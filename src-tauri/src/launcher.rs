//! 引导壳多模式：主程序（main）/ 安装向导（--installer）/ 卸载器（--uninstaller）
//!                 / 修复模式（--repair）/ 无人值守部署（--unattended）
//!
//! 启动参数由 std::env::args 解析一次并存入 OnceLock：
//! - 无参           => main（正常主程序：托盘 / 单实例 / 窗口记忆全部启用）
//! - --installer    => installer（安装向导：拷贝自身 + 运行内嵌 Inno 静默逻辑包）
//! - --uninstaller  => uninstaller（卸载器：调起同目录 unins000.exe 静默卸载）
//! - --repair       => repair（修复模式：校验程序文件 / 重建 dirs.json / 修复快捷方式与关联）
//! - --unattended   => unattended（无人值守：读同目录 unattend.json 静默安装，无 UI 无窗口）
//!
//! 安装/卸载/修复模式下不建托盘、不做单实例激活、不还原窗口记忆，
//! 窗口复用 tauri.conf 的 "main" 窗口，由前端按 get_launch_mode 路由页面。
//! unattended 模式在进入 Tauri 运行时前直接完成安装并退出进程。

use serde_json::json;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// 内嵌 Inno 静默逻辑包（jiwei-logic.exe）。
/// 当前为 1 字节占位文件，集成阶段替换为真实逻辑包后重新编译即可。
const LOGIC_PACKAGE: &[u8] = include_bytes!("../resources/jiwei-logic.exe");

const APP_EXE_NAME: &str = "jiwei-toolbox.exe";
const LOGIC_TEMP_NAME: &str = "jw_logic.exe";

// ---------------------------------------------------------------------------
// 启动模式
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LaunchMode {
    Main,
    Installer,
    Uninstaller,
    Repair,
    Unattended,
}

impl LaunchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LaunchMode::Main => "main",
            LaunchMode::Installer => "installer",
            LaunchMode::Uninstaller => "uninstaller",
            LaunchMode::Repair => "repair",
            LaunchMode::Unattended => "unattended",
        }
    }
}

static LAUNCH_MODE: OnceLock<LaunchMode> = OnceLock::new();

/// 解析并缓存启动模式（进程生命周期内只解析一次）
pub fn detect_launch_mode() -> LaunchMode {
    *LAUNCH_MODE.get_or_init(|| {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.iter().any(|a| a == "--installer") {
            LaunchMode::Installer
        } else if args.iter().any(|a| a == "--uninstaller") {
            LaunchMode::Uninstaller
        } else if args.iter().any(|a| a == "--repair") {
            LaunchMode::Repair
        } else if args.iter().any(|a| a == "--unattended") {
            LaunchMode::Unattended
        } else {
            LaunchMode::Main
        }
    })
}

/// 是否主程序模式（lib.rs 据此决定单实例 / 托盘 / 窗口记忆）
pub fn is_main_mode() -> bool {
    detect_launch_mode() == LaunchMode::Main
}

/// 前端获取当前启动模式："main" | "installer" | "uninstaller" | "repair" | "unattended"
#[tauri::command]
pub fn get_launch_mode() -> String {
    detect_launch_mode().as_str().to_string()
}

// ---------------------------------------------------------------------------
// 安装向导
// ---------------------------------------------------------------------------

static INSTALL_CANCEL: AtomicBool = AtomicBool::new(false);
static INSTALL_RUNNING: AtomicBool = AtomicBool::new(false);
static INSTALL_DIR: Mutex<Option<String>> = Mutex::new(None);
static INSTALL_DATA_DIR: Mutex<Option<String>> = Mutex::new(None);

/// 弹出系统目录选择框，返回所选路径；取消返回空字符串
#[tauri::command]
pub fn inst_choose_dir() -> String {
    rfd::FileDialog::new()
        .set_title("选择安装目录")
        .pick_folder()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 开始安装（后台线程执行，进度通过 installer-progress / installer-done 事件推送）：
/// 0) install_webv2=true 且未装 WebView2 时：先下载并静默安装 WebView2
///    （进度由 webview2 模块转发为 installer-progress {stage:"webview2"}）
/// 1) 分块拷贝自身到 <dir>\jiwei-toolbox.exe，按字节数回报 0..40 进度（此阶段可 inst_cancel）
/// 2) 创建用户数据目录（<data_dir>\配置 | \备份导出 | \缓存）
/// 3) 释放内嵌逻辑包到 %TEMP%\jw_logic.exe 并静默运行（追加 /DATA= /STARTMENU=），
///    等待期间回报 40..90 缓动假进度
/// 4) 逻辑包退出码 0 => installer-done {ok:true, dir}，否则 {ok:false, err}
#[tauri::command]
pub fn inst_start(
    app: AppHandle,
    dir: String,
    data_dir: String,
    desktop: bool,
    start_menu: bool,
    admin: bool,
    install_webv2: bool,
) -> Result<(), String> {
    if dir.trim().is_empty() {
        return Err("安装目录不能为空".into());
    }
    if INSTALL_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("安装正在进行中".into());
    }
    INSTALL_CANCEL.store(false, Ordering::SeqCst);
    if let Ok(mut slot) = INSTALL_DIR.lock() {
        *slot = Some(dir.clone());
    }
    if let Ok(mut slot) = INSTALL_DATA_DIR.lock() {
        *slot = Some(data_dir.clone());
    }

    std::thread::spawn(move || {
        let r = run_install(Some(app.clone()), &dir, &data_dir, desktop, start_menu, admin, install_webv2);
        match r {
            Ok(()) => {
                let _ = app.emit("installer-done", json!({ "ok": true, "dir": dir }));
            }
            Err(e) => {
                let _ = app.emit("installer-done", json!({ "ok": false, "dir": dir, "err": e }));
            }
        }
        INSTALL_RUNNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

/// 查询路径所在盘的剩余可用空间（字节）。
/// 路径不必存在：向上回溯到最近存在的祖先目录，再取其盘符根。
/// 获取失败返回 0（前端按"未知"处理）。
#[tauri::command]
pub fn inst_disk_free(path: String) -> u64 {
    // 自有 FFI 声明：避免为单个 API 追加 windows-sys feature
    #[repr(C)]
    struct U64(u64);
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            lp_directory_name: *const u16,
            lp_free_bytes_available_to_caller: *mut U64,
            lp_total_number_of_bytes: *mut U64,
            lp_total_number_of_free_bytes: *mut U64,
        ) -> i32;
    }

    // 回溯到存在的祖先（最多到盘根）
    let mut probe = PathBuf::from(path.trim());
    while !probe.exists() {
        match probe.parent() {
            Some(p) if p != probe => probe = p.to_path_buf(),
            _ => break,
        }
    }
    let s = probe.to_string_lossy();
    // 提取盘符根（"X:"）
    let root: String = if s.len() >= 2 && s.as_bytes()[1] == b':' {
        format!("{}\\", &s[..2])
    } else {
        return 0;
    };
    let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut free = U64(0);
        let mut total = U64(0);
        let mut total_free = U64(0);
        if GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) != 0 {
            free.0
        } else {
            0
        }
    }
}

/// 请求取消安装（仅拷贝阶段生效；逻辑包阶段为 Inno 静默安装，不可中断）
#[tauri::command]
pub fn inst_cancel() {
    INSTALL_CANCEL.store(true, Ordering::SeqCst);
}

/// 安装收尾：按需启动主程序 / 打开用户数据配置目录，然后退出安装壳
#[tauri::command]
pub fn inst_finish(run: bool, open_cfg: bool) {
    let dir = INSTALL_DIR.lock().ok().and_then(|s| s.clone());
    let data_dir = INSTALL_DATA_DIR.lock().ok().and_then(|s| s.clone());

    if run {
        if let Some(dir) = &dir {
            let exe = PathBuf::from(dir).join(APP_EXE_NAME);
            // spawn 后立刻 drop Child：子进程独立于安装壳继续运行
            let _ = Command::new(&exe).spawn();
        }
    }
    if open_cfg {
        // 双路径模型：优先打开数据目录下的配置子目录
        let cfg = match &data_dir {
            Some(d) if !d.trim().is_empty() => PathBuf::from(d).join("配置"),
            _ => crate::modules::common::user_documents()
                .join(crate::modules::common::APP_DIR_NAME)
                .join("配置"),
        };
        let _ = std::fs::create_dir_all(&cfg);
        let _ = Command::new("explorer").arg(&cfg).spawn();
    }
    std::process::exit(0);
}

fn emit_progress(app: Option<&AppHandle>, stage: &str, pct: i32, msg: &str) {
    if let Some(app) = app {
        let _ = app.emit(
            "installer-progress",
            json!({ "stage": stage, "pct": pct, "msg": msg }),
        );
    }
}

/// 执行完整安装流程（inst_start 后台线程与 --unattended 共用）。
/// app 为 None 时（无人值守）不推送任何事件，错误经返回值上报。
/// 成功返回 Ok(())，失败返回 Err(原因)。
fn run_install(
    app: Option<AppHandle>,
    dir: &str,
    data_dir: &str,
    desktop: bool,
    start_menu: bool,
    admin: bool,
    install_webv2: bool,
) -> Result<(), String> {
    // ---- 阶段 0：WebView2 运行时（按需，进度经 webview2 模块转发） ----
    if install_webv2 && !crate::modules::common::has_webview2() {
        match app.as_ref() {
            Some(a) => {
                crate::modules::webview2::set_forward_to_installer(true);
                let r = crate::modules::webview2::ensure_webview2_blocking(a);
                crate::modules::webview2::set_forward_to_installer(false);
                if let Err(err) = r {
                    return Err(format!("WebView2 安装失败: {err}"));
                }
            }
            None => {
                return Err(
                    "未安装 WebView2 运行时（无人值守部署请预先安装 WebView2 后重试）".into(),
                );
            }
        }
    }

    // ---- 阶段 1：拷贝自身 ----
    let target = PathBuf::from(dir).join(APP_EXE_NAME);
    copy_self_with_progress(app.as_ref(), &target)?;

    // ---- 阶段 2：创建用户数据目录（config / backup / cache 派生子目录） ----
    if !data_dir.trim().is_empty() {
        std::fs::create_dir_all(data_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
        for sub in ["配置", "备份导出", "缓存"] {
            let _ = std::fs::create_dir_all(PathBuf::from(data_dir).join(sub));
        }
    }

    // ---- 阶段 3：释放并运行内嵌逻辑包 ----
    let logic_path = std::env::temp_dir().join(LOGIC_TEMP_NAME);
    std::fs::write(&logic_path, LOGIC_PACKAGE).map_err(|e| format!("释放逻辑包失败: {e}"))?;

    emit_progress(app.as_ref(), "logic", 40, "正在安装运行组件…");

    let mut cmd = Command::new(&logic_path);
    cmd.arg("/VERYSILENT")
        .arg("/SUPPRESSMSGBOXES")
        .arg(format!("/DIR={}", dir))
        .arg(format!("/DESKTOP={}", if desktop { 1 } else { 0 }))
        .arg(format!("/STARTMENU={}", if start_menu { 1 } else { 0 }))
        .arg(format!("/ADMIN={}", if admin { 1 } else { 0 }));
    if !data_dir.trim().is_empty() {
        cmd.arg(format!("/DATA={}", data_dir));
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| {
            let _ = std::fs::remove_file(&logic_path);
            format!("无法启动逻辑包: {e}")
        })?;

    // 等待逻辑包退出，期间每 300ms 缓动推送 40→90 假进度
    let status = loop {
        if let Ok(Some(st)) = child.try_wait() {
            break st;
        }
        if let Ok(mut slot) = FAKE_PCT.lock() {
            *slot = (*slot + ((90 - *slot) / 8).max(1)).min(90);
            emit_progress(app.as_ref(), "logic", *slot, "正在安装运行组件…");
        }
        std::thread::sleep(Duration::from_millis(300));
    };
    let _ = std::fs::remove_file(&logic_path);

    let code = status.code().unwrap_or(-1);
    if code == 0 {
        emit_progress(app.as_ref(), "done", 100, "安装完成");
        Ok(())
    } else {
        Err(format!("安装程序退出码 {code}"))
    }
}

static FAKE_PCT: Mutex<i32> = Mutex::new(40);

/// 分块拷贝自身到 target，按已拷贝字节回报 0..40 进度；
/// 每个分块前检查取消标记，取消时清理半成品并返回 Err
fn copy_self_with_progress(app: Option<&AppHandle>, target: &PathBuf) -> Result<(), String> {
    let self_path = std::env::current_exe().map_err(|e| format!("定位自身失败: {e}"))?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建安装目录失败: {e}"))?;
    }

    let mut src = std::fs::File::open(&self_path).map_err(|e| format!("打开自身失败: {e}"))?;
    let total = src.metadata().map(|m| m.len()).unwrap_or(0);
    let mut dst =
        std::fs::File::create(target).map_err(|e| format!("创建目标文件失败: {e}"))?;

    let mut buf = vec![0u8; 256 * 1024];
    let mut copied: u64 = 0;
    loop {
        if INSTALL_CANCEL.load(Ordering::SeqCst) {
            drop(dst);
            let _ = std::fs::remove_file(target);
            return Err("用户已取消安装".into());
        }
        let n = src.read(&mut buf).map_err(|e| format!("读取自身失败: {e}"))?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).map_err(|e| format!("写入目标失败: {e}"))?;
        copied += n as u64;
        let pct = if total > 0 {
            ((copied * 40) / total) as i32
        } else {
            40
        };
        emit_progress(app, "copy", pct.min(40), "正在复制程序文件…");
    }
    dst.flush().map_err(|e| format!("写入目标失败: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 卸载器
// ---------------------------------------------------------------------------

/// 开始卸载（后台线程执行）：
/// 1) 写注册表 HKCU\Software\极维工具箱\Installer\KeepData 供 Inno 卸载段决定是否保留数据
/// 2) 静默运行同目录 unins000.exe（壳与卸载器同在 {app}）
/// 3) 推送 uninstaller-progress {stage:"done"} 与 uninstaller-done {ok}
#[tauri::command]
pub fn unins_start(app: AppHandle, keep_data: bool) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let shell_dir = std::env::current_exe()
        .map_err(|e| format!("定位自身失败: {e}"))?
        .parent()
        .ok_or("定位自身目录失败")?
        .to_path_buf();
    let unins = shell_dir.join("unins000.exe");
    if !unins.exists() {
        return Err(format!("未找到卸载器: {}", unins.display()));
    }

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(r"Software\极维工具箱\Installer")
        .map_err(|e| format!("写注册表失败: {e}"))?;
    key.set_value("KeepData", &(if keep_data { 1u32 } else { 0u32 }))
        .map_err(|e| format!("写注册表失败: {e}"))?;

    std::thread::spawn(move || {
        let status = Command::new(&unins)
            .arg("/VERYSILENT")
            .arg("/SUPPRESSMSGBOXES")
            .status();
        let ok = matches!(&status, Ok(s) if s.code() == Some(0));
        let _ = app.emit("uninstaller-progress", json!({ "stage": "done" }));
        let _ = app.emit("uninstaller-done", json!({ "ok": ok }));
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// 修复模式（--repair）
// ---------------------------------------------------------------------------

/// 从主程序设置页重启进入修复模式：spawn 自身 --repair（detached）后退出当前进程
#[tauri::command]
pub fn relaunch_repair() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("定位自身失败: {e}"))?;
    Command::new(&exe)
        .arg("--repair")
        .spawn()
        .map_err(|e| format!("启动修复模式失败: {e}"))?;
    std::process::exit(0);
}

/// 修复项结果（repair-done 事件 items 元素）
#[derive(serde::Serialize, Clone)]
pub struct RepairItem {
    pub item: String,
    /// "ok" | "repaired" | "warn"
    pub status: String,
    pub detail: String,
}

fn repair_emit_progress(app: &AppHandle, stage: &str, pct: i32, msg: &str) {
    let _ = app.emit(
        "repair-progress",
        json!({ "stage": stage, "pct": pct, "msg": msg }),
    );
}

/// JSON 字符串转义为纯 ASCII（非 ASCII 字符转 \uXXXX，含代理对），
/// 与 jiwei-logic.iss ssPostInstall 的 JsonEscUnicode 输出格式一致
fn json_escape_ascii(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        let v = c as u32;
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if v < 0x20 => out.push_str(&format!("\\u{:04x}", v)),
            _ if v <= 0x7F => out.push(c),
            _ if v <= 0xFFFF => out.push_str(&format!("\\u{:04x}", v)),
            _ => {
                let vv = v - 0x10000;
                out.push_str(&format!(
                    "\\u{:04x}\\u{:04x}",
                    0xD800 + (vv >> 10),
                    0xDC00 + (vv & 0x3FF)
                ));
            }
        }
    }
    out
}

/// 定位已有安装的双路径：注册表 HKCU\...\Dirs → <exe 目录>\dirs.json → 默认值
fn repair_locate_dirs() -> (String, String) {
    if let Some(d) = crate::modules::dirs::read_legacy_dirs() {
        if !d.program.trim().is_empty() && !d.data.trim().is_empty() {
            return (d.program, d.data);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if let Ok(bytes) = std::fs::read(parent.join("dirs.json")) {
                let text =
                    crate::modules::common::strip_bom(String::from_utf8_lossy(&bytes).into_owned());
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    let program = v
                        .get("program")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let data = v
                        .get("data")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !program.trim().is_empty() && !data.trim().is_empty() {
                        return (program, data);
                    }
                }
            }
        }
    }
    let program = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let data = crate::modules::common::local_appdata()
        .join(crate::modules::common::APP_DIR_NAME)
        .to_string_lossy()
        .into_owned();
    (program, data)
}

/// 重建 dirs.json（ssPostInstall 同款纯 ASCII \uXXXX 格式，UTF-8 无 BOM）：
/// 写入 <exe 目录>\dirs.json 与 <data>\配置\dirs.json；仅创建配置文件，不改动数据内容
fn rebuild_dirs_json(program: &str, data: &str) -> Result<(), String> {
    if data.trim().is_empty() {
        return Err("数据目录未知，无法重建目录配置".into());
    }
    let d = PathBuf::from(data);
    let j = format!(
        "{{\"program\":\"{}\",\"data\":\"{}\",\"config\":\"{}\",\"backup\":\"{}\",\"cache\":\"{}\",\"log\":\"{}\"}}",
        json_escape_ascii(program),
        json_escape_ascii(data),
        json_escape_ascii(&d.join("配置").to_string_lossy()),
        json_escape_ascii(&d.join("备份导出").to_string_lossy()),
        json_escape_ascii(&d.join("缓存").to_string_lossy()),
        json_escape_ascii(&d.join("缓存").join("logs").to_string_lossy()),
    );
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            std::fs::write(parent.join("dirs.json"), &j)
                .map_err(|e| format!("写入 dirs.json 失败: {e}"))?;
        }
    }
    let cfg = d.join("配置");
    std::fs::create_dir_all(&cfg).map_err(|e| format!("创建配置目录失败: {e}"))?;
    std::fs::write(cfg.join("dirs.json"), &j).map_err(|e| format!("写入 dirs.json 失败: {e}"))?;
    Ok(())
}

/// 开始修复（后台线程执行，进度经 repair-progress 推送，结果经 repair-done 推送）：
/// 1) check   校验程序 exe 存在且大小 >5MB（损坏时仅警告：运行中无法自替换）
/// 2) dirs    dirs.json 缺失/不可解析时按 ssPostInstall 格式重建（不改动数据目录内容）
/// 3) shortcut 重跑内嵌逻辑包（重建快捷方式 + 注册表关联）
/// 4) done    推送 repair-done {ok, items}
#[tauri::command]
pub fn repair_start(app: AppHandle) -> Result<(), String> {
    std::thread::spawn(move || run_repair(app));
    Ok(())
}

fn run_repair(app: AppHandle) {
    let mut items: Vec<RepairItem> = Vec::new();
    let (program, data) = repair_locate_dirs();

    // ---- 阶段 1：check ----
    repair_emit_progress(&app, "check", 10, "正在校验程序文件…");
    let exe = std::env::current_exe().ok();
    let exe_size = exe.as_ref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len());
    match (exe.as_ref(), exe_size) {
        (Some(p), Some(size)) if size > 5 * 1024 * 1024 => {
            items.push(RepairItem {
                item: "程序文件校验".into(),
                status: "ok".into(),
                detail: p.to_string_lossy().into_owned(),
            });
        }
        _ => {
            items.push(RepairItem {
                item: "程序文件校验".into(),
                status: "warn".into(),
                detail: "程序文件缺失或疑似损坏（运行中无法自替换），请重新运行完整安装包覆盖安装"
                    .into(),
            });
        }
    }
    let dirs_json_ok = exe
        .as_ref()
        .and_then(|p| p.parent())
        .map(|d| d.join("dirs.json"))
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| {
            serde_json::from_str::<serde_json::Value>(&crate::modules::common::strip_bom(
                String::from_utf8_lossy(&b).into_owned(),
            ))
            .is_ok()
        })
        .unwrap_or(false);

    // ---- 阶段 2：dirs ----
    repair_emit_progress(&app, "dirs", 30, "正在检查目录配置…");
    if dirs_json_ok {
        items.push(RepairItem {
            item: "目录配置".into(),
            status: "ok".into(),
            detail: "dirs.json 存在且可解析".into(),
        });
    } else {
        match rebuild_dirs_json(&program, &data) {
            Ok(()) => items.push(RepairItem {
                item: "目录配置".into(),
                status: "repaired".into(),
                detail: format!("已重建 dirs.json（program={} data={}）", program, data),
            }),
            Err(e) => items.push(RepairItem {
                item: "目录配置".into(),
                status: "warn".into(),
                detail: e,
            }),
        }
    }

    // ---- 阶段 3：shortcut（重跑内嵌逻辑包，重建快捷方式与文件关联） ----
    repair_emit_progress(&app, "shortcut", 40, "正在修复快捷方式与文件关联…");
    let mut ok = true;
    if LOGIC_PACKAGE.len() < 1024 {
        items.push(RepairItem {
            item: "快捷方式与文件关联".into(),
            status: "warn".into(),
            detail: "内嵌逻辑包未集成，跳过快捷方式与文件关联修复".into(),
        });
    } else {
        let logic_path = std::env::temp_dir().join(LOGIC_TEMP_NAME);
        match std::fs::write(&logic_path, LOGIC_PACKAGE)
            .map_err(|e| format!("释放逻辑包失败: {e}"))
            .and_then(|_| {
                Command::new(&logic_path)
                    .arg("/VERYSILENT")
                    .arg("/SUPPRESSMSGBOXES")
                    .arg(format!("/DIR={}", program))
                    .arg(format!("/DATA={}", data))
                    .arg("/DESKTOP=1")
                    .arg("/STARTMENU=1")
                    .spawn()
                    .map_err(|e| format!("无法启动逻辑包: {e}"))
            }) {
            Ok(mut child) => {
                // 等待期间缓动推送 40→90 假进度
                let status = loop {
                    if let Ok(Some(st)) = child.try_wait() {
                        break st;
                    }
                    if let Ok(mut slot) = FAKE_PCT.lock() {
                        *slot = (*slot + ((90 - *slot) / 8).max(1)).min(90);
                        repair_emit_progress(&app, "shortcut", *slot, "正在修复快捷方式与文件关联…");
                    }
                    std::thread::sleep(Duration::from_millis(300));
                };
                let code = status.code().unwrap_or(-1);
                if code == 0 {
                    items.push(RepairItem {
                        item: "快捷方式与文件关联".into(),
                        status: "ok".into(),
                        detail: "快捷方式与文件关联已修复".into(),
                    });
                } else {
                    ok = false;
                    items.push(RepairItem {
                        item: "快捷方式与文件关联".into(),
                        status: "warn".into(),
                        detail: format!("逻辑包退出码 {code}，修复未完成"),
                    });
                }
            }
            Err(e) => {
                ok = false;
                items.push(RepairItem {
                    item: "快捷方式与文件关联".into(),
                    status: "warn".into(),
                    detail: e,
                });
            }
        }
        let _ = std::fs::remove_file(&logic_path);
    }

    // ---- 阶段 4：done ----
    repair_emit_progress(&app, "done", 100, "修复完成");
    let _ = app.emit(
        "repair-done",
        json!({
            "ok": ok,
            "items": items.iter().map(|i| serde_json::to_value(i).unwrap_or_default()).collect::<Vec<_>>(),
        }),
    );
}

// ---------------------------------------------------------------------------
// 无人值守部署（--unattended）
// ---------------------------------------------------------------------------

/// 无人值守日志：`<data>\缓存\logs\unattend-YYYYMMDD.log`（UTF-8 追加，格式仿 op_log）
fn unattend_log(data_dir: &str, module: &str, action: &str, detail: &str) {
    let now = chrono::Local::now();
    let dir = PathBuf::from(data_dir).join("缓存").join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("unattend-{}.log", now.format("%Y%m%d")));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] [{}] {} : {}", now.format("%H:%M:%S"), module, action, detail);
    }
}

/// 无人值守入口（lib.rs 在进入 Tauri 运行时前调用，无 UI）：
/// 读 exe 同目录 unattend.json（UTF-8 / UTF-8-BOM 均可）→ run_install 静默安装
/// → run_after=true 则启动主程序。返回进程退出码：
/// 0 成功 / 1 配置缺失或非法 / 2 安装失败
pub fn run_unattended() -> i32 {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[unattended] 定位自身失败: {e}");
            return 1;
        }
    };
    let exe_dir = match exe.parent() {
        Some(d) => d.to_path_buf(),
        None => {
            eprintln!("[unattended] 定位自身目录失败");
            return 1;
        }
    };
    let cfg_path = exe_dir.join("unattend.json");
    let bytes = match std::fs::read(&cfg_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[unattended] 读取 {} 失败: {e}", cfg_path.display());
            return 1;
        }
    };
    let text = crate::modules::common::strip_bom(String::from_utf8_lossy(&bytes).into_owned());
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[unattended] unattend.json 解析失败: {e}");
            return 1;
        }
    };
    let program = v
        .get("program")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let data = v
        .get("data")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if program.is_empty() || data.is_empty() {
        eprintln!("[unattended] unattend.json 缺少必填字段 program / data");
        return 1;
    }
    let get_bool = |k: &str, d: bool| v.get(k).and_then(|x| x.as_bool()).unwrap_or(d);
    let desktop = get_bool("desktop", true);
    let start_menu = get_bool("start_menu", true);
    let admin = get_bool("admin", false);
    let install_webv2 = get_bool("install_webv2", true);
    let run_after = get_bool("run_after", false);

    unattend_log(&data, "unattend", "开始无人值守安装", &format!("program={} data={}", program, data));

    let r = run_install(None, &program, &data, desktop, start_menu, admin, install_webv2);
    match &r {
        Ok(()) => unattend_log(&data, "unattend", "无人值守安装完成", ""),
        Err(e) => unattend_log(&data, "unattend", "无人值守安装失败", e),
    }

    if r.is_ok() && run_after {
        let _ = Command::new(PathBuf::from(&program).join(APP_EXE_NAME)).spawn();
    }
    if r.is_err() {
        return 2;
    }
    0
}

// ---------------------------------------------------------------------------
// 版本升级预填（安装向导）
// ---------------------------------------------------------------------------

/// 已有安装检测结果（安装向导预填用）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingInstall {
    pub exists: bool,
    pub program: String,
    pub data: String,
    pub old_version: String,
}

/// 读取旧版本号：HKLM Uninstall 键 DisplayVersion（Inno {AppId}_is1）→ dirs.json 旁 version.txt
fn read_old_version() -> String {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE};
    use winreg::RegKey;

    const UNINST_SUB: &str =
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{8E7B6C2A-4D3F-4A1E-9C5B-2F6D8A0E1B7C}_is1";
    for path in [UNINST_SUB, &format!(r"SOFTWARE\WOW6432Node\{}", UNINST_SUB)] {
        if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(path, KEY_QUERY_VALUE)
        {
            if let Ok(v) = k.get_value::<String, _>("DisplayVersion") {
                if !v.trim().is_empty() {
                    return v;
                }
            }
        }
    }
    // dirs.json 旁 version.txt（<exe 目录>\version.txt）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if let Ok(bytes) = std::fs::read(parent.join("version.txt")) {
                let s = crate::modules::common::strip_bom(
                    String::from_utf8_lossy(&bytes).into_owned(),
                );
                return s.trim().to_string();
            }
        }
    }
    String::new()
}

/// 检测已有安装（注册表 Dirs 六值 + 版本号），供安装向导步骤 2 预填与提示条
#[tauri::command]
pub fn inst_detect_existing() -> ExistingInstall {
    let mut program = String::new();
    let mut data = String::new();
    if let Some(d) = crate::modules::dirs::read_legacy_dirs() {
        program = d.program;
        data = d.data;
    }
    // 注册表缺失时兜底读 <exe 目录>\dirs.json
    if program.trim().is_empty() || data.trim().is_empty() {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                if let Ok(bytes) = std::fs::read(parent.join("dirs.json")) {
                    let text = crate::modules::common::strip_bom(
                        String::from_utf8_lossy(&bytes).into_owned(),
                    );
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        let p = v
                            .get("program")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        let d = v
                            .get("data")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        if program.trim().is_empty() && !p.trim().is_empty() {
                            program = p;
                        }
                        if data.trim().is_empty() && !d.trim().is_empty() {
                            data = d;
                        }
                    }
                }
            }
        }
    }
    let old_version = read_old_version();
    let exists = !program.trim().is_empty() || !data.trim().is_empty();
    ExistingInstall { exists, program, data, old_version }
}
