//! 日志读取（log 目录）与操作日志写入

use crate::dirs::Dirs;
use std::io::Write;
use std::path::Path;

/// 格式化一条操作日志行：`[HH:MM:SS] [module] action : detail`
fn op_log_line(now: chrono::DateTime<chrono::Local>, module: &str, action: &str, detail: &str) -> String {
    format!("[{}] [{}] {} : {}", now.format("%H:%M:%S"), module, action, detail)
}

/// 追加一条操作日志：`<log 目录>\ops-YYYYMMDD.log`（UTF-8，格式 `[HH:MM:SS] [module] action : detail`）
pub fn op_log(dirs: &Dirs, module: &str, action: &str, detail: &str) {
    let now = chrono::Local::now();
    let path = Path::new(&dirs.log).join(format!("ops-{}.log", now.format("%Y%m%d")));
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}", op_log_line(now, module, action, detail));
    }
}

/// 在资源管理器中打开日志目录
#[tauri::command]
pub fn open_log_dir() -> Result<(), String> {
    let dirs = crate::dirs::load_dirs();
    std::fs::create_dir_all(&dirs.log).map_err(|e| e.to_string())?;
    std::process::Command::new("explorer")
        .arg(&dirs.log)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 列出日志文件名（按名称倒序）
#[tauri::command]
pub fn list_logs() -> Vec<String> {
    let dirs = crate::dirs::load_dirs();
    let mut out: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dirs.log) {
        for e in entries.flatten() {
            if e.path().is_file() {
                out.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out.reverse();
    out
}

/// 读取日志内容（仅允许 log 目录下的普通文件名，防目录穿越）
#[tauri::command]
pub fn read_log(file: String) -> Result<String, String> {
    if file.contains('\\') || file.contains('/') || file.contains("..") {
        return Err("非法日志文件名".into());
    }
    let dirs = crate::dirs::load_dirs();
    let path = std::path::Path::new(&dirs.log).join(&file);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {}", file, e))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 清空日志目录（删除全部日志文件，保留目录本身），返回删除的文件数
#[tauri::command]
pub fn clear_logs() -> Result<usize, String> {
    let dirs = crate::dirs::load_dirs();
    std::fs::create_dir_all(&dirs.log).map_err(|e| e.to_string())?;
    let mut count = 0usize;
    let entries = std::fs::read_dir(&dirs.log).map_err(|e| e.to_string())?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_file() {
            if std::fs::remove_file(&p).is_ok() {
                count += 1;
            }
        }
    }
    op_log(&dirs, "logs", "clear_logs", &format!("清空日志目录（{} 个文件）", count));
    Ok(count)
}

// ---------------------------------------------------------------------------
// 日志保留策略（settings.json log_retention，0=关闭，支持 7/15/30 天）
// ---------------------------------------------------------------------------

/// 读取日志保留天数（0 = 关闭；非法值回退 0）
pub fn log_retention_days() -> u32 {
    crate::common::read_setting_raw("log_retention")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(0)
}

/// 读取日志保留天数
#[tauri::command]
pub fn get_log_retention() -> u32 {
    log_retention_days()
}

/// 设置日志保留天数（0=关闭，7/15/30 天），持久化 settings.json
#[tauri::command]
pub fn set_log_retention(days: u32) -> Result<(), String> {
    if !matches!(days, 0 | 7 | 15 | 30) {
        return Err("保留天数仅支持 0（关闭）/ 7 / 15 / 30".into());
    }
    crate::common::write_setting_raw("log_retention", serde_json::json!(days))
}

/// 保留天数文件筛选判断：mtime 距 now 超过 days*86400 秒视为过期。
/// days=0（关闭）恒为 false；mtime 在未来（时钟回拨）不删除。
fn log_expired(modified: std::time::SystemTime, now: std::time::SystemTime, days: u32) -> bool {
    if days == 0 {
        return false;
    }
    let max_age = std::time::Duration::from_secs(days as u64 * 86400);
    now.duration_since(modified)
        .ok()
        .map(|age| age >= max_age)
        .unwrap_or(false)
}

/// 按保留天数清理过期日志：log 目录下 mtime 超过 days*86400 秒的文件删除
/// 返回删除的文件数（days=0 直接返回 0）
pub fn cleanup_old_logs(days: u32) -> usize {
    if days == 0 {
        return 0;
    }
    let dirs = crate::dirs::load_dirs();
    let now = std::time::SystemTime::now();
    let mut count = 0usize;
    if let Ok(entries) = std::fs::read_dir(&dirs.log) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let expired = e
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| log_expired(t, now, days))
                .unwrap_or(false);
            if expired && std::fs::remove_file(&p).is_ok() {
                count += 1;
            }
        }
    }
    if count > 0 {
        op_log(&dirs, "logs", "auto_clean", &format!("自动清理 {} 个超过 {} 天的日志文件", count, days));
    }
    count
}

/// 启动时后台执行日志保留策略（lib.rs setup 主模式下调用）
pub fn startup_retention() {
    std::thread::spawn(|| {
        cleanup_old_logs(log_retention_days());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn op_log_line_format() {
        let t = chrono::Local.with_ymd_and_hms(2026, 10, 18, 9, 5, 3).unwrap();
        assert_eq!(op_log_line(t, "dirs", "set_dirs", "program=A data=B"), "[09:05:03] [dirs] set_dirs : program=A data=B");
        // detail 为空时保留尾部 " : "
        assert_eq!(op_log_line(t, "install", "tasks_done", ""), "[09:05:03] [install] tasks_done : ");
    }

    #[test]
    fn log_expired_by_retention_days() {
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        // 恰好 30 天前 → 过期（>=）
        let old = now - std::time::Duration::from_secs(30 * 86400);
        assert!(log_expired(old, now, 30));
        // 29 天前 → 保留
        let fresh = now - std::time::Duration::from_secs(29 * 86400);
        assert!(!log_expired(fresh, now, 30));
        // 7 天保留：8 天前过期、6 天前保留
        assert!(log_expired(now - std::time::Duration::from_secs(8 * 86400), now, 7));
        assert!(!log_expired(now - std::time::Duration::from_secs(6 * 86400), now, 7));
        // 刚创建的文件不过期
        assert!(!log_expired(now, now, 7));
    }

    #[test]
    fn log_expired_disabled_or_future_mtime() {
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        // days=0（策略关闭）恒不删
        assert!(!log_expired(now - std::time::Duration::from_secs(365 * 86400), now, 0));
        // mtime 在未来（时钟回拨）不删
        let future = now + std::time::Duration::from_secs(86400);
        assert!(!log_expired(future, now, 7));
    }
}
