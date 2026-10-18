//! 结构化日志与错误码表
//!
//! 日志按天落盘为 JSON Lines（每行一个 `LogEntry` JSON 对象）：
//! `<log 目录>\structured-YYYYMMDD.jsonl`，UTF-8 无 BOM。
//! 字段固定，便于按错误码 / 模块 / 等级 / 时间检索，界面直接按列渲染。

// 命令与错误码常量由界面/其它模块按需调用，未在本模块内引用不算问题
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// 等级
// ---------------------------------------------------------------------------

/// 日志等级：信息 / 警告 / 错误 / 致命
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Level {
    Info,
    Warn,
    Error,
    Fatal,
}

impl Level {
    /// 序列化后的大写字符串（与 JSON 里的 level 字段一致）
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
            Level::Fatal => "FATAL",
        }
    }
}

/// 把界面传来的等级字符串解析为 `Level`，大小写不敏感；无法识别返回 None
pub fn level_from_str(s: &str) -> Option<Level> {
    match s.trim().to_ascii_lowercase().as_str() {
        "info" => Some(Level::Info),
        "warn" | "warning" => Some(Level::Warn),
        "error" => Some(Level::Error),
        "fatal" => Some(Level::Fatal),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

/// 一条结构化日志
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogEntry {
    /// 本地时间，毫秒精度：`2026-10-18 13:30:05.123`
    pub timestamp: String,
    pub level: Level,
    /// 中文模块名，如 "软件库"、"卸载"
    pub module: String,
    /// 错误码，可为空
    #[serde(default)]
    pub error_code: String,
    /// 人类可读描述
    pub message: String,
    /// 原始错误详情，可为空
    #[serde(default)]
    pub detail: String,
    /// 关联任务 id，可为空
    #[serde(default)]
    pub task_id: String,
    /// 等待/运行中/成功/失败，可为空
    #[serde(default)]
    pub status: String,
    /// 写日志时是否为管理员
    #[serde(default)]
    pub is_admin: bool,
    /// 系统版本摘要
    #[serde(default)]
    pub os_info: String,
}

// ---------------------------------------------------------------------------
// 错误码表
// ---------------------------------------------------------------------------

pub const MOD01_001: &str = "MOD01-001";
pub const MOD01_002: &str = "MOD01-002";
pub const MOD01_003: &str = "MOD01-003";
pub const MOD02_001: &str = "MOD02-001";
pub const MOD02_002: &str = "MOD02-002";
pub const MOD03_001: &str = "MOD03-001";
pub const MOD03_002: &str = "MOD03-002";
pub const MOD04_001: &str = "MOD04-001";
pub const MOD04_002: &str = "MOD04-002";
pub const MOD05_001: &str = "MOD05-001";
pub const MOD06_001: &str = "MOD06-001";
pub const MOD06_002: &str = "MOD06-002";
pub const MOD07_001: &str = "MOD07-001";
pub const MOD07_002: &str = "MOD07-002";
pub const MOD08_001: &str = "MOD08-001";
pub const MOD08_002: &str = "MOD08-002";

/// 全部错误码：(错误码, 等级, 描述, 提示文案)
pub const ERROR_CODES: &[(&str, Level, &str, &str)] = &[
    (
        MOD01_001,
        Level::Error,
        "网络连接失败",
        "无法连接软件源，请检查网络或代理设置",
    ),
    (
        MOD01_002,
        Level::Error,
        "安装包哈希校验不匹配",
        "文件校验失败，安装包可能损坏或被篡改，将自动重试下载",
    ),
    (
        MOD01_003,
        Level::Warn,
        "磁盘剩余空间不足",
        "目标磁盘空间不足，无法下载安装包，请清理磁盘",
    ),
    (
        MOD02_001,
        Level::Warn,
        "文件被进程占用",
        "部分文件正在被程序占用，无法删除，可关闭程序重试",
    ),
    (
        MOD02_002,
        Level::Fatal,
        "注册表项权限拒绝",
        "无权限操作注册表，需要管理员权限启动工具箱",
    ),
    (
        MOD03_001,
        Level::Error,
        "备份包文件损坏",
        "备份包校验失败，标记为损坏，禁止还原操作",
    ),
    (
        MOD03_002,
        Level::Error,
        "备份密码错误",
        "密码不匹配，无法打开加密备份包",
    ),
    (
        MOD04_001,
        Level::Warn,
        "硬件传感器读取异常",
        "部分传感器信息读取失败，可能是硬件驱动限制",
    ),
    (
        MOD04_002,
        Level::Fatal,
        "硬件超温，测试自动终止",
        "硬件温度超过安全阈值，压力测试已自动停止",
    ),
    (
        MOD05_001,
        Level::Warn,
        "Hosts 写入权限不足",
        "需要管理员权限修改 Hosts 文件",
    ),
    (
        MOD06_001,
        Level::Error,
        "配置文件解析失败",
        "配置文件损坏，无法导入",
    ),
    (
        MOD06_002,
        Level::Warn,
        "导入配置存在冲突",
        "部分配置项冲突，已按导入策略合并/覆盖",
    ),
    (
        MOD07_001,
        Level::Error,
        "更新包下载失败",
        "网络中断，更新包下载失败",
    ),
    (
        MOD07_002,
        Level::Fatal,
        "更新替换失败，自动回滚",
        "文件替换失败，已回滚到旧版本，软件可正常使用",
    ),
    (
        MOD08_001,
        Level::Warn,
        "任务中断保存成功",
        "检测到程序异常关闭，任务状态已保存，下次打开可恢复任务",
    ),
    (
        MOD08_002,
        Level::Fatal,
        "文件读写失败",
        "目录权限不足，无法写入文件",
    ),
];

/// 按错误码查表：返回 (等级, 描述, 提示文案)；未知错误码返回 None
pub fn code_meta(code: &str) -> Option<(Level, &'static str, &'static str)> {
    let key = code.trim();
    ERROR_CODES
        .iter()
        .find(|t| t.0 == key)
        .map(|t| (t.1, t.2, t.3))
}

// ---------------------------------------------------------------------------
// 纯函数：JSONL 行、关键词匹配
// ---------------------------------------------------------------------------

/// 序列化为单行 JSON（JSON 内部换行会被转义，结果不含裸 `\n`）
pub fn to_jsonl_line(entry: &LogEntry) -> Result<String, String> {
    serde_json::to_string(entry).map_err(|e| format!("日志序列化失败: {}", e))
}

/// 关键词匹配：空关键词视为命中；否则在 message + detail 中做不区分大小写包含匹配
pub fn matches_keyword(entry: &LogEntry, keyword: &str) -> bool {
    let kw = keyword.trim();
    if kw.is_empty() {
        return true;
    }
    let kw = kw.to_lowercase();
    entry.message.to_lowercase().contains(&kw) || entry.detail.to_lowercase().contains(&kw)
}

// ---------------------------------------------------------------------------
// 路径与时间辅助
// ---------------------------------------------------------------------------

fn now_timestamp() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S%.3f")
        .to_string()
}

/// 日期归一：合法 8 位数字（或 `YYYY-MM-DD`）转为 YYYYMMDD，其余（含空串）取今天
fn resolve_date(date: &str) -> String {
    let t = date.trim();
    let compact = if t.len() == 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] == b'-' {
        t.replace('-', "")
    } else {
        t.to_string()
    };
    if compact.len() == 8 && compact.chars().all(|c| c.is_ascii_digit()) {
        compact
    } else {
        chrono::Local::now().format("%Y%m%d").to_string()
    }
}

/// 当天结构化日志文件：`<log 目录>\structured-YYYYMMDD.jsonl`
fn log_path(date: &str) -> PathBuf {
    Path::new(&crate::dirs::load_dirs().log).join(format!("structured-{}.jsonl", resolve_date(date)))
}

/// 从文件末尾往前按块读取，返回末尾至多 `limit` 行（保持文件中的先后顺序）
fn read_tail_lines(path: &Path, limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let len = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return Vec::new(),
    };
    if len == 0 {
        return Vec::new();
    }
    const CHUNK: u64 = 64 * 1024;
    let mut pos = len;
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut newlines = 0usize;
    let want = limit.saturating_add(1); // 多读一行，抵消文件末尾换行带来的空行
    while pos > 0 && newlines < want {
        let size = CHUNK.min(pos);
        pos -= size;
        let mut chunk = vec![0u8; size as usize];
        if file.seek(SeekFrom::Start(pos)).is_err() || file.read_exact(&mut chunk).is_err() {
            break;
        }
        newlines += chunk.iter().filter(|b| **b == b'\n').count();
        chunks.push(chunk);
    }
    let mut buf: Vec<u8> = Vec::new();
    for chunk in chunks.into_iter().rev() {
        buf.extend_from_slice(&chunk);
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .filter(|l| !l.trim().is_empty())
        .collect();
    // 未读到文件开头时，缓冲区首行可能是被截断的半行，丢弃
    if pos > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    if lines.len() > limit {
        lines.drain(0..lines.len() - limit);
    }
    lines
}

/// 读取某天的日志（解析失败的行静默跳过），返回末尾至多 `limit` 条，按时间正序
fn read_entries(date: &str, limit: usize) -> Vec<LogEntry> {
    read_tail_lines(&log_path(date), limit)
        .iter()
        .filter_map(|l| serde_json::from_str::<LogEntry>(l).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// 内部辅助（供其它模块调用）
// ---------------------------------------------------------------------------

/// 判断当前是否管理员（直接复用 common 模块的检测实现）
pub fn is_admin() -> bool {
    crate::modules::common::is_admin()
}

/// 系统版本摘要，取自硬件模块的 OS 信息：`Windows 11 专业版 (Build 22631)`
/// 该调用会走一次 CIM 查询，较慢，因此首次取到后缓存；取不到返回空串。
fn os_summary() -> String {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let info = crate::modules::hardware::hw_info_sync();
            let os = match info.get("os") {
                Some(v) => v,
                None => return String::new(),
            };
            let text = |key: &str| -> String {
                os.get(key)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };
            let caption = text("caption");
            let build = text("build");
            match (caption.is_empty(), build.is_empty()) {
                (true, _) => String::new(),
                (false, true) => caption,
                (false, false) => format!("{} (Build {})", caption, build),
            }
        })
        .clone()
}

/// 便捷写入：不需要填全字段时用它，写失败静默忽略（日志不能影响主流程）
pub fn log_simple(module: &str, level: Level, code: &str, message: &str, detail: &str) {
    let entry = LogEntry {
        timestamp: now_timestamp(),
        level,
        module: module.to_string(),
        error_code: code.to_string(),
        message: message.to_string(),
        detail: detail.to_string(),
        task_id: String::new(),
        status: String::new(),
        is_admin: is_admin(),
        os_info: os_summary(),
    };
    let _ = append_line(&entry);
}

/// 追加一行的实际落盘实现（UTF-8 无 BOM，行尾 `\n`）
fn append_line(entry: &LogEntry) -> Result<(), String> {
    let today = chrono::Local::now().format("%Y%m%d").to_string();
    let path = log_path(&today);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建日志目录失败: {}", e))?;
    }
    let line = to_jsonl_line(entry)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("打开日志文件失败: {}", e))?;
    f.write_all(format!("{}\n", line).as_bytes())
        .map_err(|e| format!("写入日志失败: {}", e))?;
    f.flush().map_err(|e| format!("写入日志失败: {}", e))
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

/// 列出某天的结构化日志：`date` 空取今天（YYYYMMDD），按时间正序（老→新），最多 `limit` 条。
#[tauri::command]
pub fn list_structured_logs(date: String, limit: usize) -> Vec<LogEntry> {
    let limit = if limit == 0 { 500 } else { limit };
    read_entries(&date, limit)
}

/// 追加一条结构化日志到当天文件
#[tauri::command]
pub fn append_structured_log(entry: LogEntry) -> Result<(), String> {
    append_line(&entry)
}

/// 多条件检索：任一筛选参数为 None / 空串则不过滤
#[tauri::command]
pub fn query_structured_logs(
    level: Option<String>,
    module: Option<String>,
    code: Option<String>,
    keyword: Option<String>,
    date: String,
    limit: usize,
) -> Vec<LogEntry> {
    let limit = if limit == 0 { 500 } else { limit };
    let level = level.as_deref().map(str::trim).filter(|s| !s.is_empty()).and_then(level_from_str);
    let module = module.unwrap_or_default().trim().to_string();
    let code = code.unwrap_or_default().trim().to_string();
    let keyword = keyword.unwrap_or_default().trim().to_string();

    let mut hits: Vec<LogEntry> = read_entries(&date, usize::MAX)
        .into_iter()
        .filter(|e| level.map(|lv| e.level == lv).unwrap_or(true))
        .filter(|e| module.is_empty() || e.module.contains(&module))
        .filter(|e| code.is_empty() || e.error_code.eq_ignore_ascii_case(&code))
        .filter(|e| matches_keyword(e, &keyword))
        .collect();
    // 命中过多时保留最近的 limit 条，仍按时间正序返回
    if hits.len() > limit {
        hits.drain(0..hits.len() - limit);
    }
    hits
}

/// 某天各等级条数统计
#[tauri::command]
pub fn structured_log_stats(date: String) -> serde_json::Value {
    let entries = read_entries(&date, usize::MAX);
    let count = |lv: Level| entries.iter().filter(|e| e.level == lv).count();
    let (info, warn, error, fatal) = (
        count(Level::Info),
        count(Level::Warn),
        count(Level::Error),
        count(Level::Fatal),
    );
    serde_json::json!({
        "info": info,
        "warn": warn,
        "error": error,
        "fatal": fatal,
        "total": entries.len(),
    })
}

/// 错误码表（供界面展示）：`{ code, level, desc, hint }`
#[tauri::command]
pub fn error_code_table() -> Vec<serde_json::Value> {
    ERROR_CODES
        .iter()
        .map(|(code, level, desc, hint)| {
            serde_json::json!({
                "code": code,
                "level": level.as_str(),
                "desc": desc,
                "hint": hint,
            })
        })
        .collect()
}

/// 删除某天的日志文件（date 空取今天），返回删除的字节数；文件不存在返回 0
#[tauri::command]
pub fn clear_structured_logs(date: String) -> Result<usize, String> {
    let path = log_path(&date);
    let size = match std::fs::metadata(&path) {
        Ok(m) => m.len() as usize,
        Err(_) => return Ok(0),
    };
    std::fs::remove_file(&path).map_err(|e| format!("删除日志文件失败: {}", e))?;
    Ok(size)
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry() -> LogEntry {
        LogEntry {
            timestamp: "2026-10-18 13:30:05.123".to_string(),
            level: Level::Warn,
            module: "卸载".to_string(),
            error_code: MOD02_001.to_string(),
            message: "文件被占用：Foo Bar.dll".to_string(),
            detail: "Access Denied (os error 5)".to_string(),
            task_id: "t-001".to_string(),
            status: "失败".to_string(),
            is_admin: false,
            os_info: "Windows 11 专业版 (Build 22631)".to_string(),
        }
    }

    #[test]
    fn level_serializes_uppercase() {
        assert_eq!(serde_json::to_string(&Level::Info).unwrap(), "\"INFO\"");
        assert_eq!(serde_json::to_string(&Level::Warn).unwrap(), "\"WARN\"");
        assert_eq!(serde_json::to_string(&Level::Error).unwrap(), "\"ERROR\"");
        assert_eq!(serde_json::to_string(&Level::Fatal).unwrap(), "\"FATAL\"");
        // 反序列化同样识别大写
        let lv: Level = serde_json::from_str("\"FATAL\"").unwrap();
        assert_eq!(lv, Level::Fatal);
    }

    #[test]
    fn all_codes_resolve() {
        assert_eq!(ERROR_CODES.len(), 16);
        for &(code, expect_level, expect_desc, expect_hint) in ERROR_CODES {
            let (level, desc, hint) = code_meta(code).expect("错误码应能查到");
            assert_eq!(level, expect_level);
            assert_eq!(desc, expect_desc);
            assert_eq!(hint, expect_hint);
            assert!(!desc.trim().is_empty());
            assert!(!hint.trim().is_empty());
        }
    }

    #[test]
    fn unknown_code_returns_none() {
        assert!(code_meta("MOD99-999").is_none());
        assert!(code_meta("").is_none());
        assert!(code_meta("mod01-001").is_none(), "错误码大小写敏感");
    }

    #[test]
    fn entry_json_roundtrip() {
        let entry = sample_entry();
        let text = serde_json::to_string(&entry).unwrap();
        let back: LogEntry = serde_json::from_str(&text).unwrap();
        assert_eq!(back.timestamp, entry.timestamp);
        assert_eq!(back.level, entry.level);
        assert_eq!(back.module, entry.module);
        assert_eq!(back.error_code, entry.error_code);
        assert_eq!(back.message, entry.message);
        assert_eq!(back.detail, entry.detail);
        assert_eq!(back.task_id, entry.task_id);
        assert_eq!(back.status, entry.status);
        assert_eq!(back.is_admin, entry.is_admin);
        assert_eq!(back.os_info, entry.os_info);
    }

    #[test]
    fn level_filter_is_case_insensitive() {
        assert_eq!(level_from_str("warn"), Some(Level::Warn));
        assert_eq!(level_from_str("WARN"), Some(Level::Warn));
        assert_eq!(level_from_str("Warn"), Some(Level::Warn));
        assert_eq!(level_from_str(" info "), Some(Level::Info));
        assert_eq!(level_from_str("ERROR"), Some(Level::Error));
        assert_eq!(level_from_str("fatal"), Some(Level::Fatal));
        assert_eq!(level_from_str("debug"), None);
        assert_eq!(level_from_str(""), None);
    }

    #[test]
    fn keyword_match_ignores_case() {
        let entry = sample_entry();
        // 英文大小写不敏感
        assert!(matches_keyword(&entry, "foo"));
        assert!(matches_keyword(&entry, "FOO BAR"));
        assert!(matches_keyword(&entry, "access denied"));
        // 中文包含匹配
        assert!(matches_keyword(&entry, "占用"));
        assert!(matches_keyword(&entry, "文件被占用"));
        // 不命中
        assert!(!matches_keyword(&entry, "网络"));
        // 空关键词（含空白）不过滤
        assert!(matches_keyword(&entry, ""));
        assert!(matches_keyword(&entry, "   "));
    }

    #[test]
    fn jsonl_line_has_no_newline() {
        let mut entry = sample_entry();
        // 消息里带真实换行，序列化后必须被转义，不能破坏 JSONL 结构
        entry.detail = "第一行\n第二行\r\n第三行".to_string();
        let line = to_jsonl_line(&entry).unwrap();
        assert!(!line.contains('\n'));
        assert!(!line.contains('\r'));
        // 仍可解析回同一内容
        let back: LogEntry = serde_json::from_str(&line).unwrap();
        assert_eq!(back.detail, entry.detail);
        // 正常条目一行一条
        let normal = to_jsonl_line(&sample_entry()).unwrap();
        assert!(!normal.contains('\n'));
        assert!(normal.starts_with('{') && normal.ends_with('}'));
    }
}

/// 现场探针：真实写一条、读回来、再按条件查询，最后清理。
/// `cargo test --lib live_structured_log -- --ignored --nocapture`
#[cfg(test)]
mod live_probe {
    use super::{clear_structured_logs, list_structured_logs, log_simple, Level};

    #[test]
    #[ignore]
    fn live_structured_log() {
        let today = chrono::Local::now().format("%Y%m%d").to_string();
        // 先写三条不同等级
        log_simple("测试模块", Level::Info, "", "探针：信息级日志", "");
        log_simple("测试模块", Level::Warn, "", "探针：警告级日志", "detail-warn");
        log_simple("测试模块", Level::Error, "MOD01-001", "探针：错误级日志", "网络超时");

        let list = list_structured_logs(String::new(), 100);
        println!("read back {} entries", list.len());
        for e in list.iter().rev().take(4) {
            println!(
                "  {} [{}] {} {} {}",
                e.timestamp, e.level.as_str(), e.module, e.error_code, e.message
            );
        }
        assert!(!list.is_empty(), "刚写入的日志应该能读回来");
        // 三条探针都要在
        let probes = list.iter().filter(|e| e.message.starts_with("探针：")).count();
        println!("probe entries={probes}");
        assert!(probes >= 3, "写入的 3 条探针日志都应读回，实际 {probes}");
        // 带错误码的那条要能查到码
        assert!(
            list.iter().any(|e| e.error_code == "MOD01-001"),
            "错误码应被保留"
        );

        // 清理，避免污染用户日志
        let freed = clear_structured_logs(today);
        println!("cleared = {freed:?}");
    }
}
