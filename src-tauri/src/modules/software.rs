//! 软件清单与批量静默安装（下载 → 校验 → 静默安装，串行队列 + 断点续传）

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::Emitter;

static INSTALL_CANCEL: AtomicBool = AtomicBool::new(false);
static INSTALL_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// 队列暂停标记（下载分块间原子检查）
static TASK_PAUSED: AtomicBool = AtomicBool::new(false);

/// 队列运行状态快照（queue-state 事件与 get_queue_state 返回）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueState {
    pub running: bool,
    pub paused: bool,
    pub current_id: String,
    /// 整体进度：已完成项数 / 总项数 * 100（当前项内部进度由 install-progress 推送）
    pub total_pct: i64,
    pub total: usize,
    pub done: usize,
}

static TASK_STATE: Mutex<QueueState> = Mutex::new(QueueState {
    running: false,
    paused: false,
    current_id: String::new(),
    total_pct: 0,
    total: 0,
    done: 0,
});

/// 更新状态快照并推送 queue-state 事件
fn publish_queue_state(app: &tauri::AppHandle, running: bool, current_id: &str, done: usize, total: usize) {
    let paused = TASK_PAUSED.load(Ordering::Relaxed);
    let mut guard = TASK_STATE.lock().unwrap_or_else(|e| e.into_inner());
    guard.running = running;
    guard.paused = paused;
    guard.current_id = current_id.to_string();
    guard.total = total;
    guard.done = done;
    guard.total_pct = if total > 0 { (done as i64 * 100 / total as i64).min(100) } else { 0 };
    let _ = app.emit("queue-state", serde_json::json!({
        "running": guard.running,
        "paused": guard.paused,
        "current_id": guard.current_id,
        "total_pct": guard.total_pct,
    }));
}

/// 暂停任务队列（下载分块间的原子暂停点生效）
#[tauri::command]
pub fn pause_tasks(app: tauri::AppHandle) {
    TASK_PAUSED.store(true, Ordering::SeqCst);
    let (cur, done, total) = {
        let g = TASK_STATE.lock().unwrap_or_else(|e| e.into_inner());
        (g.current_id.clone(), g.done, g.total)
    };
    publish_queue_state(&app, true, &cur, done, total);
    let dirs = crate::dirs::load_dirs();
    crate::logs::op_log(&dirs, "install", "pause_tasks", "队列已暂停");
}

/// 恢复任务队列
#[tauri::command]
pub fn resume_tasks(app: tauri::AppHandle) {
    TASK_PAUSED.store(false, Ordering::SeqCst);
    let (cur, done, total) = {
        let g = TASK_STATE.lock().unwrap_or_else(|e| e.into_inner());
        (g.current_id.clone(), g.done, g.total)
    };
    publish_queue_state(&app, true, &cur, done, total);
    let dirs = crate::dirs::load_dirs();
    crate::logs::op_log(&dirs, "install", "resume_tasks", "队列已恢复");
}

/// 终止任务队列（原子终止 + 可选删除 cache\downloads 中的下载缓存）
#[tauri::command]
pub fn terminate_tasks(app: tauri::AppHandle, delete_cache: bool) {
    TASK_PAUSED.store(false, Ordering::SeqCst);
    INSTALL_CANCEL.store(true, Ordering::SeqCst);
    // 立即推进度到终止态，让前端 UI 即刻响应
    let done = {
        let mut g = TASK_STATE.lock().unwrap_or_else(|e| e.into_inner());
        g.paused = false;
        g.done
    };
    publish_queue_state(&app, false, "", done, 0);
    if delete_cache {
        let dirs = crate::dirs::load_dirs();
        let dl_dir = std::path::Path::new(&dirs.cache).join("downloads");
        let _ = std::fs::remove_dir_all(&dl_dir);
        crate::logs::op_log(&dirs, "install", "terminate_tasks", "队列已终止，下载缓存已删除");
    } else {
        let dirs = crate::dirs::load_dirs();
        crate::logs::op_log(&dirs, "install", "terminate_tasks", "队列已终止");
    }
}

/// 查询当前队列状态
#[tauri::command]
pub fn get_queue_state() -> QueueState {
    TASK_STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 设置全部任务完成后的动作：none | notify | log | shutdown | hibernate（持久化 settings.json）
#[tauri::command]
pub fn set_task_complete_action(action: String) -> Result<(), String> {
    if !matches!(action.as_str(), "none" | "notify" | "log" | "shutdown" | "hibernate") {
        return Err(format!("未知的完成动作: {}", action));
    }
    crate::common::write_setting_raw("task_complete_action", serde_json::Value::String(action))
}

/// 执行"全部任务完成"动作
fn run_complete_action(app: &tauri::AppHandle, dirs: &crate::dirs::Dirs) {
    let action = crate::common::read_setting_raw("task_complete_action")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "none".into());
    match action.as_str() {
        "notify" => {
            // 前端监听 tasks-done 弹通知
            let _ = app.emit("tasks-done", ());
        }
        "log" => {
            crate::logs::op_log(dirs, "install", "tasks_done", "全部任务已完成");
        }
        "shutdown" => {
            // 关机（60 秒缓冲，可 shutdown /a 取消）。仅由用户显式设置触发。
            crate::logs::op_log(dirs, "install", "tasks_done", "全部任务完成，60 秒后关机");
            let _ = std::process::Command::new("shutdown").args(["/s", "/t", "60"]).spawn();
        }
        "hibernate" => {
            crate::logs::op_log(dirs, "install", "tasks_done", "全部任务完成，系统休眠");
            let _ = std::process::Command::new("shutdown").arg("/h").spawn();
        }
        _ => {}
    }
}

/// 软件清单更新日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogEntry {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub notes: String,
}

/// 软件清单条目
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Software {
    pub id: String,
    pub name: String,
    pub category: String,
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub silent_args: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub notes: String,
    /// 一句话简介
    #[serde(default)]
    pub description: String,
    /// 估算安装包/安装体积（MB）
    #[serde(rename = "size_mb", default)]
    pub size_mb: f64,
    /// "free" | "open" | "commercial"
    #[serde(default)]
    pub license: String,
    /// 官网
    #[serde(default)]
    pub homepage: String,
    /// 更新日志（示意，1-2 条）
    #[serde(default)]
    pub changelog: Vec<ChangelogEntry>,
    /// 依赖的其他软件 id
    #[serde(default)]
    pub depends: Vec<String>,
    /// MD5 校验（预留，当前留空）
    #[serde(default)]
    pub md5: String,
}

/// 内置默认占位清单（softwares.json 缺失时兜底，3 条）
fn default_software() -> Vec<Software> {
    let mk = |id: &str, name: &str, cat: &str, url: &str, args: &str, kind: &str, notes: &str| Software {
        id: id.into(),
        name: name.into(),
        category: cat.into(),
        url: url.into(),
        sha256: String::new(),
        silent_args: args.into(),
        kind: kind.into(),
        notes: notes.into(),
        description: String::new(),
        size_mb: 0.0,
        license: "free".into(),
        homepage: String::new(),
        changelog: Vec::new(),
        depends: Vec::new(),
        md5: String::new(),
    };
    vec![
        mk("placeholder-1", "占位软件 1", "系统工具", "https://example.com/setup1.exe", "/S", "exe", "内置占位清单：请放置 resources\\softwares.json"),
        mk("placeholder-2", "占位软件 2", "浏览器", "https://example.com/setup2.exe", "/S", "exe", "内置占位清单：请放置 resources\\softwares.json"),
        mk("placeholder-3", "占位软件 3", "系统工具", "https://example.com/setup3.msi", "/qn /norestart", "msi", "内置占位清单：请放置 resources\\softwares.json"),
    ]
}

/// 候选清单路径：exe 相对 resources → 本地缓存
fn software_list_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // 安装后：{app} 目录下 resources 子目录
            v.push(dir.join("resources").join("softwares.json"));
            // 开发期：src-tauri/target/{debug|release}/ 与项目根的多级组合
            v.push(dir.join("..").join("resources").join("softwares.json"));
            v.push(dir.join("..").join("..").join("resources").join("softwares.json"));
            v.push(dir.join("..").join("..").join("..").join("resources").join("softwares.json"));
        }
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        v.push(PathBuf::from(&la).join(crate::common::APP_DIR_NAME).join("缓存").join("softwares.json"));
    }
    v
}

fn list_software_inner() -> Vec<Software> {
    for p in software_list_paths() {
        if let Ok(bytes) = fs::read(&p) {
            let text = crate::common::strip_bom(String::from_utf8_lossy(&bytes).into_owned());
            if let Ok(list) = serde_json::from_str::<Vec<Software>>(&text) {
                return list;
            }
        }
    }
    // 兜底：内嵌构建期清单（resources/softwares.json 随二进制打包，任何位置可用）
    let text = crate::common::strip_bom(include_str!("../../resources/softwares.json").to_string());
    serde_json::from_str::<Vec<Software>>(&text).unwrap_or_else(|_| default_software())
}

/// 获取软件清单
#[tauri::command]
pub fn list_software() -> Vec<Software> {
    list_software_inner()
}

/// 内置远程清单源（可经 settings.json 的 software_source_url 覆盖）
const DEFAULT_SOFTWARE_SOURCE: &str = "https://raw.githubusercontent.com/jiwei-toolbox/softwares/main/softwares.json";

/// 远程清单更新结果
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateListResult {
    pub count: usize,
    pub source: String,
}

/// 校验清单 JSON 文本并补齐可选字段，返回软件列表：
/// - 顶层必须是 JSON 数组
/// - 每条必须有非空 id / name / url
/// - 可选字段缺失时补默认值，保证结果可被 list_software 解析
fn validate_software_list(text: &str) -> Result<Vec<Software>, String> {
    let text = crate::common::strip_bom(text.to_string());
    let raw: Vec<serde_json::Value> = serde_json::from_str(&text)
        .map_err(|e| format!("清单不是合法 JSON 数组: {}", e))?;

    let mut list: Vec<Software> = Vec::with_capacity(raw.len());
    for (i, mut item) in raw.into_iter().enumerate() {
        for key in ["id", "name", "url"] {
            let ok = item.get(key).and_then(|v| v.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false);
            if !ok {
                return Err(format!("清单第 {} 条缺少必填字段: {}", i + 1, key));
            }
        }
        if let Some(obj) = item.as_object_mut() {
            obj.entry("category".to_string()).or_insert(serde_json::json!("未分类"));
            obj.entry("sha256".to_string()).or_insert(serde_json::json!(""));
            obj.entry("silentArgs".to_string()).or_insert(serde_json::json!(""));
            obj.entry("type".to_string()).or_insert(serde_json::json!("exe"));
            obj.entry("notes".to_string()).or_insert(serde_json::json!(""));
            obj.entry("description".to_string()).or_insert(serde_json::json!(""));
            obj.entry("size_mb".to_string()).or_insert(serde_json::json!(0));
            obj.entry("license".to_string()).or_insert(serde_json::json!("free"));
            obj.entry("homepage".to_string()).or_insert(serde_json::json!(""));
            obj.entry("changelog".to_string()).or_insert(serde_json::json!([]));
            obj.entry("depends".to_string()).or_insert(serde_json::json!([]));
            obj.entry("md5".to_string()).or_insert(serde_json::json!(""));
        }
        let sw: Software = serde_json::from_value(item)
            .map_err(|e| format!("清单第 {} 条结构无效: {}", i + 1, e))?;
        list.push(sw);
    }
    Ok(list)
}

/// 从远程源拉取软件清单 → 校验结构 → 写入缓存 softwares.json
/// （清单校验失败 / 网络失败 / 404 均返回 Err，本地清单仍可用）
#[tauri::command]
pub fn update_software_list() -> Result<UpdateListResult, String> {
    let mut url = crate::common::get_setting("software_source_url".into());
    if url.trim().is_empty() {
        url = DEFAULT_SOFTWARE_SOURCE.into();
    }
    let dirs = crate::dirs::load_dirs();
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("网络客户端初始化失败: {}", e))?;
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("网络请求失败: {}", e))?;
    if !resp.status().is_success() {
        // 404 等非 2xx 一律视为"源未就绪"
        return Err(format!("软件源未就绪: HTTP {}", resp.status()));
    }
    let text = resp.text().map_err(|e| format!("读取响应失败: {}", e))?;
    let list = validate_software_list(&text)?;

    let dest = PathBuf::from(&dirs.cache).join("softwares.json");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建缓存目录失败: {}", e))?;
    }
    let out = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
    std::fs::write(&dest, out).map_err(|e| format!("写入缓存清单失败: {}", e))?;
    crate::logs::op_log(&dirs, "install", "update_list", &format!("更新远程清单 {} 条 <- {}", list.len(), url));
    Ok(UpdateListResult { count: list.len(), source: "remote".into() })
}

/// 取消当前安装队列
#[tauri::command]
pub fn cancel_install() {
    INSTALL_CANCEL.store(true, Ordering::Relaxed);
}

fn emit_progress(app: Option<&tauri::AppHandle>, id: &str, stage: &str, pct: i64, msg: &str) {
    let Some(app) = app else { return };
    let _ = app.emit(
        "install-progress",
        serde_json::json!({ "id": id, "stage": stage, "pct": pct, "msg": msg }),
    );
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut f, &mut hasher).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// 下载（HTTP Range 断点续传；206 追加 / 200 重来）
/// limiter：Some 时对每个读块做共享令牌桶限速（并发下载线程共享同一限速器）
fn download_with_resume(
    url: &str,
    dest: &Path,
    app: Option<&tauri::AppHandle>,
    id: &str,
    limiter: Option<&crate::taskopts::RateLimiter>,
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| e.to_string())?;

    let mut pos: u64 = dest.metadata().map(|m| m.len()).unwrap_or(0);
    loop {
        if INSTALL_CANCEL.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        wait_if_paused();
        let mut req = client.get(url);
        if pos > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={}-", pos));
        }
        let mut resp = req.send().map_err(|e| format!("下载请求失败: {}", e))?;

        // 已下载完整但服务器拒绝 Range
        if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && pos > 0 {
            return Ok(());
        }
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        // 服务器不支持断点续传（返回 200）→ 从头下载
        if pos > 0 && resp.status() == reqwest::StatusCode::OK {
            pos = 0;
        }

        let total = resp.content_length().map(|l| l + pos);
        let mut file = if pos > 0 {
            OpenOptions::new().append(true).open(dest)
        } else {
            File::create(dest)
        }
        .map_err(|e| format!("写入文件失败: {}", e))?;

        let mut downloaded = pos;
        let mut buf = [0u8; 64 * 1024];
        let mut last_pct: i64 = -1;
        loop {
            if INSTALL_CANCEL.load(Ordering::Relaxed) {
                return Err("已取消".into());
            }
            // 原子暂停点：下载分块间检查，暂停期间挂起（仍响应终止）
            while TASK_PAUSED.load(Ordering::Relaxed) {
                if INSTALL_CANCEL.load(Ordering::Relaxed) {
                    return Err("已取消".into());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            let n = resp.read(&mut buf).map_err(|e| format!("下载中断: {}", e))?;
            if n == 0 {
                break;
            }
            // 共享令牌桶限速：预算不足时阻塞（并发线程共享同一限速器）
            if let Some(l) = limiter {
                l.consume(n as u64);
            }
            file.write_all(&buf[..n]).map_err(|e| format!("写入文件失败: {}", e))?;
            downloaded += n as u64;
            let pct = total.map(|t| (downloaded * 100 / t) as i64).unwrap_or(-1);
            if pct != last_pct {
                last_pct = pct;
                emit_progress(app, id, "download", pct, &format!("已下载 {} MB / 总计 {}", downloaded / 1048576, total.map(|t| format!("{} MB", t / 1048576)).unwrap_or_else(|| "未知".into())));
            }
        }

        if total.map(|t| downloaded >= t).unwrap_or(true) {
            return Ok(());
        }
        // 未下完（连接中断）→ 从当前位置继续
        pos = downloaded;
    }
}

/// 暂停等待辅助：暂停期间每 200ms 检查一次终止标记
fn wait_if_paused() {
    while TASK_PAUSED.load(Ordering::Relaxed) {
        if INSTALL_CANCEL.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// 下载阶段（失败自动重试 3 次：2s/4s/8s，断点续传续传），成功返回本地文件路径
fn download_stage(
    app: &tauri::AppHandle,
    dirs: &crate::dirs::Dirs,
    sw: &Software,
    limiter: Option<std::sync::Arc<crate::taskopts::RateLimiter>>,
) -> Result<PathBuf, String> {
    let dl_dir = Path::new(&dirs.cache).join("downloads");
    fs::create_dir_all(&dl_dir).map_err(|e| format!("创建下载目录失败: {}", e))?;
    let dest = dl_dir.join(format!("{}.exe", sw.id));

    emit_progress(Some(app), &sw.id, "download", 0, "开始下载");
    let mut dl: Result<(), String> = Err(String::new());
    for attempt in 0..=3usize {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs([2u64, 4, 8][attempt - 1]));
            if INSTALL_CANCEL.load(Ordering::Relaxed) {
                break;
            }
            emit_progress(Some(app), &sw.id, "retry", 0, &format!("第 {} 次重试", attempt));
        }
        dl = download_with_resume(&sw.url, &dest, Some(app), &sw.id, limiter.as_deref());
        if dl.is_ok() || INSTALL_CANCEL.load(Ordering::Relaxed) {
            break;
        }
    }
    match dl {
        Ok(()) => {
            crate::logs::op_log(dirs, "install", "download", &format!("{} ({}) 下载完成 <- {}", sw.name, sw.id, sw.url));
            Ok(dest)
        }
        Err(e) => Err(format!("下载失败: {}", e)),
    }
}

/// 校验（清单 sha256 非空时）+ 静默安装
fn verify_and_install(app: &tauri::AppHandle, dirs: &crate::dirs::Dirs, sw: &Software, dest: &Path) -> Result<(), String> {
    if !sw.sha256.trim().is_empty() {
        emit_progress(Some(app), &sw.id, "verify", 0, "SHA256 校验中");
        match sha256_file(dest) {
            Ok(h) if h.eq_ignore_ascii_case(sw.sha256.trim()) => {
                emit_progress(Some(app), &sw.id, "verify", 100, "校验通过");
                crate::logs::op_log(dirs, "install", "verify", &format!("{} ({}) SHA256 校验通过", sw.name, sw.id));
            }
            _ => {
                emit_progress(Some(app), &sw.id, "error", 0, "SHA256 校验失败");
                crate::logs::op_log(dirs, "install", "verify", &format!("{} ({}) SHA256 校验失败", sw.name, sw.id));
                return Err("SHA256 校验失败".into());
            }
        }
    }

    emit_progress(Some(app), &sw.id, "install", 0, "开始静默安装");
    crate::logs::op_log(dirs, "install", "install", &format!("{} ({}) 开始静默安装 ({})", sw.name, sw.id, sw.silent_args));
    let status = if sw.kind.eq_ignore_ascii_case("msi") {
        std::process::Command::new("msiexec")
            .arg("/i")
            .arg(dest)
            .args(sw.silent_args.split_whitespace())
            .status()
    } else {
        std::process::Command::new(dest)
            .args(sw.silent_args.split_whitespace())
            .status()
    };
    match status {
        Ok(s) if s.success() => {
            emit_progress(Some(app), &sw.id, "done", 100, "安装完成");
            crate::logs::op_log(dirs, "install", "result", &format!("{} ({}) 安装成功", sw.name, sw.id));
            Ok(())
        }
        Ok(s) => {
            let reason = format!("安装程序异常退出: {:?}", s.code());
            emit_progress(Some(app), &sw.id, "error", 0, &reason);
            crate::logs::op_log(dirs, "install", "result", &format!("{} ({}) {}", sw.name, sw.id, reason));
            Err(reason)
        }
        Err(e) => {
            let reason = format!("启动安装程序失败: {}", e);
            emit_progress(Some(app), &sw.id, "error", 0, &reason);
            crate::logs::op_log(dirs, "install", "result", &format!("{} ({}) {}", sw.name, sw.id, reason));
            Err(reason)
        }
    }
}

/// 串行执行单个任务项（下载 → 校验 → 安装），返回 Err(原因) 表示该项失败
fn run_single_item(
    app: &tauri::AppHandle,
    dirs: &crate::dirs::Dirs,
    sw: &Software,
    limiter: Option<std::sync::Arc<crate::taskopts::RateLimiter>>,
) -> Result<(), String> {
    let dest = download_stage(app, dirs, sw, limiter)?;
    verify_and_install(app, dirs, sw, &dest)
}

/// 安装队列线程体：按 task_opts 决定执行方式
/// - serial（默认）：1 路串行，逐项 下载→校验→安装
/// - parallel：N 路分块（chunk）处理——chunk 内并发下载（共享限速器），chunk 内安装仍串行
/// 支持暂停/终止（原子标记在下载循环与分块边界生效）
fn run_install_queue(app: tauri::AppHandle, ids: Vec<String>) {
    let dirs = crate::dirs::load_dirs();
    let opts = crate::taskopts::load_task_opts();
    let parallel = opts.mode == "parallel" && opts.concurrency > 1;
    let concurrency = if parallel { opts.concurrency as usize } else { 1 };
    let limiter = if opts.speed_limit_kb > 0 {
        Some(std::sync::Arc::new(crate::taskopts::RateLimiter::new(opts.speed_limit_kb as u64 * 1024)))
    } else {
        None
    };
    if limiter.is_some() {
        crate::logs::op_log(&dirs, "install", "task_opts", &format!(
            "任务队列启动: mode={} concurrency={} 限速 {} KB/s",
            opts.mode, concurrency, opts.speed_limit_kb
        ));
    }

    let total = ids.len();
    let mut success: Vec<String> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    publish_queue_state(&app, true, "", 0, total);

    let mut done = 0usize;
    'outer: for chunk in ids.chunks(concurrency) {
        if INSTALL_CANCEL.load(Ordering::Relaxed) {
            break;
        }
        wait_if_paused();
        if INSTALL_CANCEL.load(Ordering::Relaxed) {
            break;
        }

        // 清单查找（一次加载整个清单，chunk 内共享）
        let list = list_software_inner();
        let mut pending: Vec<(String, Software)> = Vec::with_capacity(chunk.len());
        for id in chunk {
            match list.iter().find(|s| s.id == *id) {
                Some(sw) => pending.push((id.clone(), sw.clone())),
                None => {
                    emit_progress(Some(&app), id, "error", 0, "软件清单中不存在该 id");
                    failed.push((id.clone(), "软件清单中不存在该 id".into()));
                    done += 1;
                }
            }
        }
        let chunk_first = pending.first().map(|(id, _)| id.clone()).unwrap_or_default();
        publish_queue_state(&app, true, &chunk_first, done, total);

        if concurrency == 1 {
            // 串行：逐项完整执行
            for (id, sw) in &pending {
                if INSTALL_CANCEL.load(Ordering::Relaxed) {
                    emit_progress(Some(&app), id, "skip", 0, "队列已终止");
                    break 'outer;
                }
                publish_queue_state(&app, true, id, done, total);
                match run_single_item(&app, &dirs, sw, limiter.clone()) {
                    Ok(()) => {
                        success.push(id.clone());
                        done += 1;
                    }
                    Err(reason) => {
                        if INSTALL_CANCEL.load(Ordering::Relaxed) {
                            emit_progress(Some(&app), id, "skip", 0, "队列已终止");
                            break 'outer;
                        }
                        crate::logs::op_log(&dirs, "install", "download", &format!("{} ({}) {}（已重试 3 次）", sw.name, id, reason));
                        failed.push((id.clone(), reason));
                        done += 1;
                    }
                }
            }
        } else {
            // 并行：chunk 内并发下载（暂停/终止在下载循环内生效），下载完成后再串行安装
            let handles: Vec<_> = pending
                .iter()
                .map(|(_, sw)| {
                    let app = app.clone();
                    let dirs = dirs.clone();
                    let sw = sw.clone();
                    let limiter = limiter.clone();
                    std::thread::spawn(move || download_stage(&app, &dirs, &sw, limiter))
                })
                .collect();
            let mut downloaded: Vec<(String, &Software, Result<PathBuf, String>)> = Vec::with_capacity(pending.len());
            for ((id, sw), h) in pending.iter().zip(handles) {
                let res = h.join().unwrap_or_else(|_| Err("下载线程异常退出".into()));
                downloaded.push((id.clone(), sw, res));
            }

            for (id, sw, res) in downloaded {
                if INSTALL_CANCEL.load(Ordering::Relaxed) {
                    emit_progress(Some(&app), &id, "skip", 0, "队列已终止");
                    break 'outer;
                }
                wait_if_paused();
                match res {
                    Ok(dest) => {
                        publish_queue_state(&app, true, &id, done, total);
                        match verify_and_install(&app, &dirs, sw, &dest) {
                            Ok(()) => {
                                success.push(id.clone());
                                done += 1;
                            }
                            Err(reason) => {
                                failed.push((id.clone(), reason));
                                done += 1;
                            }
                        }
                    }
                    Err(reason) => {
                        if INSTALL_CANCEL.load(Ordering::Relaxed) {
                            emit_progress(Some(&app), &id, "skip", 0, "队列已终止");
                            break 'outer;
                        }
                        emit_progress(Some(&app), &id, "error", 0, &reason);
                        crate::logs::op_log(&dirs, "install", "download", &format!("{} ({}) {}（已重试 3 次）", sw.name, id, reason));
                        failed.push((id.clone(), reason));
                        done += 1;
                    }
                }
            }
        }
    }

    // 全部完成：推送结果汇总（skipped = 取消/未处理的剩余项）
    let skipped = total.saturating_sub(success.len() + failed.len());
    let _ = app.emit(
        "install-summary",
        serde_json::json!({
            "total": total,
            "success": success,
            "failed": failed.iter().map(|(id, reason)| serde_json::json!({ "id": id, "reason": reason })).collect::<Vec<_>>(),
            "skipped": skipped,
        }),
    );
    crate::logs::op_log(&dirs, "install", "summary", &format!("批量安装结束: 共 {} 项: 成功 {} 失败 {} 跳过 {}", total, success.len(), failed.len(), skipped));
    publish_queue_state(&app, false, "", success.len() + failed.len(), total);
    TASK_PAUSED.store(false, Ordering::SeqCst);

    // 全部任务完成：按用户设置执行完成动作（notify/log/shutdown/hibernate）
    run_complete_action(&app, &dirs);
}

/// 启动任务队列（后台串行，进度经 install-progress / queue-state 事件推送）
fn spawn_task_queue(app: tauri::AppHandle, ids: Vec<String>) {
    let mut guard = INSTALL_THREAD.lock().unwrap_or_else(|e| e.into_inner());
    // 已有旧队列在跑：先终止并等待结束
    if let Some(old) = guard.take() {
        INSTALL_CANCEL.store(true, Ordering::Relaxed);
        TASK_PAUSED.store(false, Ordering::SeqCst);
        let _ = old.join();
    }
    INSTALL_CANCEL.store(false, Ordering::Relaxed);
    TASK_PAUSED.store(false, Ordering::SeqCst);
    *guard = Some(std::thread::spawn(move || run_install_queue(app, ids)));
}

/// 加入安装队列（任务队列入口，支持 pause/resume/terminate 控制）
#[tauri::command]
pub async fn queue_install(app: tauri::AppHandle, ids: Vec<String>) {
    spawn_task_queue(app, ids);
}

/// 批量静默安装（兼容旧入口，同 queue_install：后台串行队列，进度经 install-progress 事件推送）
#[tauri::command]
pub async fn install_batch(app: tauri::AppHandle, ids: Vec<String>) {
    spawn_task_queue(app, ids);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_minimal_list_fills_defaults() {
        // 仅必填字段：可选字段补默认值后应可完整解析
        let text = r#"[{"id":"app1","name":"应用一","url":"https://example.com/a.exe"}]"#;
        let list = validate_software_list(text).expect("最小清单应通过校验");
        assert_eq!(list.len(), 1);
        let sw = &list[0];
        assert_eq!(sw.id, "app1");
        assert_eq!(sw.name, "应用一");
        assert_eq!(sw.category, "未分类");
        assert_eq!(sw.kind, "exe");
        assert_eq!(sw.license, "free");
        assert_eq!(sw.size_mb, 0.0);
        assert!(sw.changelog.is_empty());
        assert!(sw.depends.is_empty());
    }

    #[test]
    fn validate_full_list() {
        // camelCase（silentArgs）与 snake_case 混用字段均按 serde rename 规则解析
        let text = concat!(
            r#"[{"id":"msi1","name":"MSI 包","category":"开发","url":"https://e.com/a.msi","#,
            r#""sha256":"deadbeef","silentArgs":"/qn","type":"msi","size_mb":120.5,"license":"open"}]"#
        );
        let list = validate_software_list(text).expect("完整清单应通过校验");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].kind, "msi");
        assert_eq!(list[0].silent_args, "/qn");
        assert_eq!(list[0].sha256, "deadbeef");
        assert!((list[0].size_mb - 120.5).abs() < f64::EPSILON);
    }

    #[test]
    fn validate_missing_required_field_is_err() {
        // 缺 id / name / url 分别报错，且错误信息指明序号与字段名
        for (json, key) in [
            (r#"[{"name":"a","url":"u"}]"#, "id"),
            (r#"[{"id":"x","url":"u"}]"#, "name"),
            (r#"[{"id":"x","name":"a"}]"#, "url"),
        ] {
            let err = validate_software_list(json).expect_err("缺必填字段应报错");
            assert!(err.contains(&format!("缺少必填字段: {}", key)), "错误信息应含字段名: {err}");
        }
        // 空串必填字段视同缺失
        let err = validate_software_list(r#"[{"id":"  ","name":"a","url":"u"}]"#).unwrap_err();
        assert!(err.contains("id"));
    }

    #[test]
    fn validate_non_array_is_err() {
        // 顶层不是数组
        assert!(validate_software_list(r#"{"id":"x"}"#).is_err());
        assert!(validate_software_list("null").is_err());
        assert!(validate_software_list("").is_err());
    }

    #[test]
    fn validate_invalid_json_is_err() {
        let err = validate_software_list("[{broken").unwrap_err();
        assert!(err.contains("清单不是合法 JSON 数组"));
    }

    #[test]
    fn validate_accepts_utf8_bom() {
        // 远程源常见 utf-8-sig 响应应可解析
        let text = "\u{feff}[{\"id\":\"bom1\",\"name\":\"B\",\"url\":\"u\"}]";
        let list = validate_software_list(text).expect("带 BOM 清单应通过校验");
        assert_eq!(list[0].id, "bom1");
    }

    #[test]
    fn validate_empty_array_ok() {
        let list = validate_software_list("[]").expect("空数组应通过校验");
        assert!(list.is_empty());
    }
}

#[cfg(test)]
mod e2e_download_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// 极简 HTTP 文件服务器（线程内跑）：支持 GET + Range（仅起点Range，206）
    fn spawn_http_server(body: Vec<u8>) -> (String, std::sync::Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let range_hits = std::sync::Arc::new(AtomicUsize::new(0));
        let hits = range_hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = vec![0u8; 4096];
                let _ = stream.read(&mut buf);
                let req = String::from_utf8_lossy(&buf);
                let lower = req.to_ascii_lowercase();
                if req.starts_with("GET") && lower.contains("range: bytes=") {
                    let start: u64 = req
                        .split("ange: bytes=")
                        .nth(1)
                        .and_then(|s| s.split('-').next())
                        .and_then(|s| s.trim().parse().ok())
                        .unwrap_or(0);
                    hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let head = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                        body.len() - start as usize,
                        start,
                        body.len() - 1,
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&body[start as usize..]);
                } else {
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&body);
                }
            }
        });
        (format!("http://{}/file.bin", addr), range_hits)
    }

    fn tmpfile(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("jw-e2e-test");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn full_download_matches() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let (url, _) = spawn_http_server(body.clone());
        let dest = tmpfile("full.bin");
        let _ = std::fs::remove_file(&dest);
        download_with_resume(&url, &dest, None, "t1", None).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn resume_from_partial() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let body: Vec<u8> = (0..80_000u32).map(|i| (i % 253) as u8).collect();
        let (url, hits) = spawn_http_server(body.clone());
        let dest = tmpfile("partial.bin");
        // 预写前半部分，模拟中断
        std::fs::write(&dest, &body[..40_000]).unwrap();
        download_with_resume(&url, &dest, None, "t2", None).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(hits.load(std::sync::atomic::Ordering::SeqCst) >= 1, "应发出 Range 请求");
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn rate_limiter_throttles() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let body = vec![0xABu8; 16 * 1024]; // 16KB
        let (url, _) = spawn_http_server(body);
        let dest = tmpfile("rate.bin");
        let _ = std::fs::remove_file(&dest);
        let limiter = crate::taskopts::RateLimiter::new(4 * 1024); // 4KB/s
        let t0 = std::time::Instant::now();
        download_with_resume(&url, &dest, None, "t3", Some(&limiter)).unwrap();
        let elapsed = t0.elapsed().as_secs_f64();
        // 16KB @ 4KB/s 应 >= 3.5s（窗口 400B，16 块 64KB——大块走 full-window 线性等待）
        assert!(elapsed >= 3.0, "限速未生效：{}s", elapsed);
        assert!(elapsed < 60.0, "限速过慢：{}s", elapsed);
        let _ = std::fs::remove_file(&dest);
    }


    #[test]
    fn concurrent_downloads_shared_limiter() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let body: Vec<u8> = (0..30_000u32).map(|i| (i % 249) as u8).collect();
        let (url1, _) = spawn_http_server(body.clone());
        let (url2, _) = spawn_http_server(body.clone());
        let dest1 = tmpfile("conc1.bin");
        let dest2 = tmpfile("conc2.bin");
        let _ = std::fs::remove_file(&dest1);
        let _ = std::fs::remove_file(&dest2);
        // 两线程共享 8KB/s 限速器：总 60KB 应 >= 6s（共享限速正确性）
        let limiter = std::sync::Arc::new(crate::taskopts::RateLimiter::new(8 * 1024));
        let h1 = std::thread::spawn({
            let url = url1.clone();
            let dest = dest1.clone();
            let lim = limiter.clone();
            move || download_with_resume(&url, &dest, None, "c1", Some(&*lim))
        });
        let h2 = std::thread::spawn({
            let url = url2.clone();
            let dest = dest2.clone();
            let lim = limiter.clone();
            move || download_with_resume(&url, &dest, None, "c2", Some(&*lim))
        });
        let r1 = h1.join().unwrap();
        let r2 = h2.join().unwrap();
        r1.unwrap();
        r2.unwrap();
        assert_eq!(std::fs::read(&dest1).unwrap(), body);
        assert_eq!(std::fs::read(&dest2).unwrap(), body);
        let _ = std::fs::remove_file(&dest1);
        let _ = std::fs::remove_file(&dest2);
    }

    #[test]
    fn cancel_aborts() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let body = vec![0x11u8; 2_000_000];
        let (url, _) = spawn_http_server(body);
        let dest = tmpfile("cancel.bin");
        let _ = std::fs::remove_file(&dest);
        INSTALL_CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
        let r = download_with_resume(&url, &dest, None, "t4", None);
        INSTALL_CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(r.is_err());
        let _ = std::fs::remove_file(&dest);
    }
}
