//! 快捷命令工具箱：预设 Windows 维护命令 + 用户自定义命令
//!
//! 执行策略：
//! - 预设命令是硬编码的 argv，不经过 shell，参数不可注入。
//! - 自定义命令由用户自己填写，必须走 cmd /c；界面侧会二次确认。
//! - 所有命令都重定向到临时文件再读，避免管道缓冲区写满导致死锁。

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// 单条输出最多保留的字节数，防止 `dir /s C:\` 之类把界面撑爆
const MAX_OUTPUT: usize = 200 * 1024;

#[derive(Serialize, Clone)]
pub struct PresetCmd {
    pub id: String,
    pub label: String,
    pub desc: String,
    /// 需要管理员权限
    pub admin: bool,
    /// 预计耗时提示
    pub hint: String,
}

/// 预设命令白名单。id → (标题, 说明, 是否需要管理员, 提示, argv)
fn preset(id: &str) -> Option<(&'static str, &'static str, bool, &'static str, Vec<&'static str>)> {
    let item = match id {
        "ipconfig" => (
            "网络配置一览",
            "查看所有网卡的 IP、网关、DNS 与 MAC 地址",
            false,
            "约 1 秒",
            vec!["ipconfig", "/all"],
        ),
        "flushdns" => (
            "刷新 DNS 缓存",
            "清除本机 DNS 解析缓存，解决域名解析异常",
            false,
            "约 1 秒",
            vec!["ipconfig", "/flushdns"],
        ),
        "netstat" => (
            "查看全部连接",
            "列出所有 TCP/UDP 连接与监听端口（含 PID）",
            false,
            "约 2 秒",
            vec!["netstat", "-ano"],
        ),
        "sfc" => (
            "系统文件检查（SFC）",
            "扫描并修复受损的 Windows 系统文件",
            true,
            "通常 3~15 分钟",
            vec!["sfc", "/scannow"],
        ),
        "dism-scan" => (
            "检查组件存储健康",
            "DISM 扫描系统映像是否损坏，只读不修复",
            true,
            "通常 1~5 分钟",
            vec!["dism", "/Online", "/Cleanup-Image", "/ScanHealth"],
        ),
        "dism-restore" => (
            "修复组件存储",
            "DISM 修复系统映像损坏，需要联网下载修复源",
            true,
            "通常 5~30 分钟",
            vec!["dism", "/Online", "/Cleanup-Image", "/RestoreHealth"],
        ),
        "chkdsk" => (
            "检查系统盘",
            "只读检查 C 盘文件系统错误，不修改磁盘",
            true,
            "通常 1~10 分钟",
            vec!["chkdsk", "C:", "/scan"],
        ),
        "systeminfo" => (
            "系统信息摘要",
            "查看系统版本、安装日期、内存、补丁等摘要",
            false,
            "约 5 秒",
            vec!["systeminfo"],
        ),
        "powercfg-battery" => (
            "电池健康报告",
            "生成本机电池损耗报告（台式机无电池会提示）",
            false,
            "约 2 秒",
            vec!["powercfg", "/batteryreport", "/output", "battery-report.html"],
        ),
        "winsat" => (
            "Windows 体验指数",
            "WinSAT 重新评估硬件性能分数",
            true,
            "约 1~3 分钟",
            vec!["winsat", "formal"],
        ),
        "driverquery" => (
            "已安装驱动列表",
            "列出全部驱动及其状态",
            false,
            "约 3 秒",
            vec!["driverquery", "/v"],
        ),
        "w32tm" => (
            "时间同步状态",
            "查看系统时间同步服务状态与上次同步结果",
            false,
            "约 2 秒",
            vec!["w32tm", "/query", "/status"],
        ),
        _ => return None,
    };
    Some(item)
}

/// 预设命令的展示顺序
const PRESET_IDS: &[&str] = &[
    "ipconfig",
    "flushdns",
    "netstat",
    "systeminfo",
    "driverquery",
    "w32tm",
    "sfc",
    "dism-scan",
    "dism-restore",
    "chkdsk",
    "winsat",
    "powercfg-battery",
];

/// 全部预设命令
#[tauri::command]
pub fn list_preset_commands() -> Vec<PresetCmd> {
    PRESET_IDS
        .iter()
        .filter_map(|id| {
            let (label, desc, admin, hint, _) = preset(id)?;
            Some(PresetCmd {
                id: id.to_string(),
                label: label.to_string(),
                desc: desc.to_string(),
                admin,
                hint: hint.to_string(),
            })
        })
        .collect()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CustomCommand {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Serialize)]
pub struct CmdResult {
    /// 退出码；-1 表示超时被终止
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    /// 耗时毫秒
    pub elapsed_ms: u64,
    pub timed_out: bool,
    /// 输出被截断
    pub truncated: bool,
}

/// 把 GBK / UTF-8 / UTF-16 的输出统一成可读文本。
/// 中文 Windows 的命令行工具大多输出 GBK（cp936），直接按 UTF-8 解会全是乱码。
fn decode_output(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    // UTF-16 LE（部分 PowerShell / 重定向场景）：ASCII 区间奇数位为 0。
    // 必须先判断再做去 NUL，否则尾部的 0x00 0x00 会把长度变奇数导致误判。
    if bytes.len() >= 4 && bytes.len() % 2 == 0 && bytes[1] == 0 && bytes[3] == 0 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let text = String::from_utf16_lossy(&units);
        return text.trim_matches('\0').trim().to_string();
    }
    // 定长缓冲读出来的尾部 NUL 要去掉
    let bytes = match bytes.iter().rposition(|b| *b != 0) {
        Some(end) => &bytes[..=end],
        None => return String::new(),
    };
    // 纯 ASCII 或合法 UTF-8 优先
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.trim().to_string();
    }
    let (text, _, _) = encoding_rs::GBK.decode(bytes);
    text.trim().to_string()
}

fn temp_file(tag: &str) -> PathBuf {
    let name = format!(
        "jiwei-cmd-{}-{}.tmp",
        tag,
        chrono::Local::now().format("%H%M%S%3f")
    );
    std::env::temp_dir().join(name)
}

fn read_limited(path: &PathBuf) -> (String, bool) {
    let mut buf = Vec::new();
    if let Ok(mut f) = std::fs::File::open(path) {
        buf.reserve(4096);
        let _ = f.by_ref().take(MAX_OUTPUT as u64 + 1).read_to_end(&mut buf);
    }
    let truncated = buf.len() > MAX_OUTPUT;
    if truncated {
        buf.truncate(MAX_OUTPUT);
    }
    (decode_output(&buf), truncated)
}

/// Windows 上 child.kill() 只终止直接子进程。走 cmd /c 时那是一层壳，
/// 真正的命令（如 ping、dism）会变成孤儿进程继续跑。用 taskkill /T 连树一起杀。
fn kill_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// 核心执行：重定向到临时文件 + 轮询超时控制。
/// 不用 output() 是因为它会在管道写满时死锁，也不支持超时。
fn run_capture(mut cmd: std::process::Command, timeout: Duration) -> Result<CmdResult, String> {
    let out_path = temp_file("out");
    let err_path = temp_file("err");
    let out_f = std::fs::File::create(&out_path).map_err(|e| format!("创建临时文件失败：{e}"))?;
    let err_f = std::fs::File::create(&err_path).map_err(|e| format!("创建临时文件失败：{e}"))?;
    cmd.stdout(std::process::Stdio::from(out_f));
    cmd.stderr(std::process::Stdio::from(err_f));
    cmd.creation_flags(CREATE_NO_WINDOW);

    let start = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&out_path);
            let _ = std::fs::remove_file(&err_path);
            return Err(format!("无法启动命令：{e}"));
        }
    };
    let child_pid = child.id();

    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    kill_tree(child_pid);
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    // taskkill 是异步的，给一点时间让子进程真正退出并刷完缓冲
                    std::thread::sleep(Duration::from_millis(200));
                    break;
                }
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(_) => break,
        }
    }
    let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
    let elapsed_ms = start.elapsed().as_millis() as u64;

    let (stdout, t1) = read_limited(&out_path);
    let (stderr, t2) = read_limited(&err_path);
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);

    Ok(CmdResult {
        code: if timed_out { -1 } else { code },
        stdout,
        stderr,
        elapsed_ms,
        timed_out,
        truncated: t1 || t2,
    })
}

/// 运行预设命令。只接受白名单 id，argv 完全由后端决定。
#[tauri::command]
pub async fn run_preset_command(id: String) -> Result<CmdResult, String> {
    let (label, _, _, _, argv) = preset(id.trim()).ok_or_else(|| "未知的预设命令".to_string())?;
    let label = label.to_string();
    let args: Vec<String> = argv.iter().skip(1).map(|s| s.to_string()).collect();
    let program = argv[0].to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new(&program);
        cmd.args(&args);
        // SFC / DISM / CHKDSK 都可能跑很久，给足 40 分钟；其余 3 分钟内必回
        let timeout = Duration::from_secs(40 * 60);
        let res = run_capture(cmd, timeout)?;
        let dirs = crate::dirs::load_dirs();
        crate::logs::op_log(
            &dirs,
            "shellcmd",
            "preset",
            &format!(
                "{} 执行完成，退出码 {}，耗时 {} ms",
                label, res.code, res.elapsed_ms
            ),
        );
        Ok(res)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 运行用户自定义命令。命令由用户输入，走 cmd /c，界面侧需要二次确认。
#[tauri::command]
pub async fn run_custom_command(command: String, timeout_secs: u64) -> Result<CmdResult, String> {
    let line = command.trim().to_string();
    if line.is_empty() {
        return Err("命令不能为空".into());
    }
    if line.len() > 2000 {
        return Err("命令过长（上限 2000 字符）".into());
    }
    let timeout = Duration::from_secs(timeout_secs.clamp(5, 1800));
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", &line]);
        let res = run_capture(cmd, timeout)?;
        let dirs = crate::dirs::load_dirs();
        crate::logs::op_log(
            &dirs,
            "shellcmd",
            "custom",
            &format!(
                "自定义命令：{}（退出码 {}）",
                line.chars().take(120).collect::<String>(),
                res.code
            ),
        );
        Ok(res)
    })
    .await
    .map_err(|e| e.to_string())?
}

const CUSTOM_KEY: &str = "custom_commands";

/// 读取用户保存的自定义命令
#[tauri::command]
pub fn list_custom_commands() -> Vec<CustomCommand> {
    crate::modules::common::read_setting_raw(CUSTOM_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// 覆盖保存自定义命令列表
#[tauri::command]
pub fn save_custom_commands(items: Vec<CustomCommand>) -> Result<(), String> {
    if items.len() > 100 {
        return Err("自定义命令最多 100 条".into());
    }
    for it in &items {
        if it.name.trim().is_empty() || it.command.trim().is_empty() {
            return Err("名称和命令都不能为空".into());
        }
        if it.command.len() > 2000 {
            return Err(format!("「{}」的命令过长", it.name));
        }
    }
    let v = serde_json::to_value(items).map_err(|e| e.to_string())?;
    crate::modules::common::write_setting_raw(CUSTOM_KEY, v)
}

#[cfg(test)]
mod tests {
    use super::{decode_output, list_preset_commands, preset};

    #[test]
    fn all_presets_resolve() {
        // list_preset_commands 依赖 preset 的 id 顺序，漏映射会拿到空 id
        let list = list_preset_commands();
        assert!(list.len() >= 10);
        for p in &list {
            assert!(!p.id.is_empty(), "预设 {} 的 id 为空", p.label);
            assert!(!p.label.is_empty());
        }
    }

    #[test]
    fn preset_whitelist_rejects_unknown() {
        assert!(preset("ipconfig").is_some());
        assert!(preset("sfc").is_some());
        assert!(preset("rm -rf /").is_none());
        assert!(preset("").is_none());
        assert!(preset("calc.exe").is_none());
    }

    #[test]
    fn preset_argv_never_contains_shell_metachars() {
        // 预设命令不经过 shell，这里再兜一层：argv 不应含 & | > <
        for id in ["ipconfig", "sfc", "dism-scan", "chkdsk", "netstat"] {
            let (_, _, _, _, argv) = preset(id).unwrap();
            for a in argv {
                assert!(
                    !a.contains('&') && !a.contains('|') && !a.contains('>') && !a.contains('<'),
                    "{id} 的 argv 含 shell 元字符: {a}"
                );
            }
        }
    }

    #[test]
    fn gbk_output_is_decoded() {
        // 中文 Windows 的 ipconfig 输出是 GBK，直接 UTF-8 解会乱码
        let gbk = b"\xd0\xc2\xce\xc4\xbc\xfe"; // 「新文件」的 GBK 编码
        assert_eq!(decode_output(gbk), "新文件");
        assert_eq!(decode_output(b"plain ascii"), "plain ascii");
        assert_eq!(decode_output("中文 UTF-8".as_bytes()), "中文 UTF-8");
        assert_eq!(decode_output(b""), "");
    }

    #[test]
    fn utf16_output_is_decoded() {
        let mut bytes = Vec::new();
        for ch in "abc".encode_utf16() {
            bytes.extend_from_slice(&ch.to_le_bytes());
        }
        assert_eq!(decode_output(&bytes), "abc");
        // 读取固定长度缓冲时常见的尾部 NUL 必须清掉
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(decode_output(&bytes), "abc");
    }

    #[test]
    fn trailing_nul_is_stripped() {
        assert_eq!(decode_output(b"hello\0\0"), "hello");
        assert_eq!(decode_output(&[0u8; 8]), "");
    }

    /// 现场探针：真跑一次 ipconfig，确认中文输出不乱码。
    /// `cargo test --lib live_run_ipconfig -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_run_ipconfig() {
        let res = tauri::async_runtime::block_on(super::run_preset_command("ipconfig".into()))
            .expect("ipconfig 应该能执行");
        println!("code={} elapsed={}ms truncated={}", res.code, res.elapsed_ms, res.truncated);
        for line in res.stdout.lines().take(8) {
            println!("  {line}");
        }
        assert_eq!(res.code, 0);
        assert!(!res.stdout.is_empty());
        // 中文 Windows 下应有「适配器」或英文 "adapter"，不能是乱码方块
        let lower = res.stdout.to_lowercase();
        assert!(
            lower.contains("adapter") || res.stdout.contains("适配器") || res.stdout.contains("以太网"),
            "输出疑似乱码：{}",
            res.stdout.chars().take(80).collect::<String>()
        );
    }

    /// 现场探针：自定义命令与超时控制
    /// `cargo test --lib live_run_custom -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_run_custom() {
        let res = tauri::async_runtime::block_on(super::run_custom_command(
            "echo 极维工具箱测试".into(),
            10,
        ))
        .expect("echo 应该能执行");
        println!("code={} out={:?}", res.code, res.stdout);
        assert_eq!(res.code, 0);
        assert!(res.stdout.contains("极维工具箱"), "GBK/UTF-8 解码失败：{:?}", res.stdout);
    }

    /// 现场探针：超时必须真的终止，且不留孤儿进程。
    /// 用 ping 127.0.0.1 -n 30（约 30 秒）配 3 秒超时。
    /// `cargo test --lib live_timeout_kills -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_timeout_kills() {
        let start = std::time::Instant::now();
        let res = tauri::async_runtime::block_on(super::run_custom_command(
            "ping 127.0.0.1 -n 30".into(),
            3,
        ))
        .expect("命令应该能启动");
        let wall = start.elapsed().as_secs();
        println!("timed_out={} code={} wall={}s", res.timed_out, res.code, wall);
        assert!(res.timed_out, "3 秒超时未触发");
        assert!(wall < 12, "超时后没有及时返回，实际耗时 {wall} 秒");
        assert_eq!(res.code, -1);
    }
}
