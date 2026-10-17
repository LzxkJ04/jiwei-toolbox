//! 日志读取（log 目录）与操作日志写入

use crate::dirs::Dirs;
use std::io::Write;
use std::path::Path;

/// 追加一条操作日志：`<log 目录>\ops-YYYYMMDD.log`（UTF-8，格式 `[HH:MM:SS] [module] action : detail`）
pub fn op_log(dirs: &Dirs, module: &str, action: &str, detail: &str) {
    let now = chrono::Local::now();
    let path = Path::new(&dirs.log).join(format!("ops-{}.log", now.format("%Y%m%d")));
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] [{}] {} : {}", now.format("%H:%M:%S"), module, action, detail);
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

/// 按保留天数清理过期日志：log 目录下 mtime 超过 days*86400 秒的文件删除
/// 返回删除的文件数（days=0 直接返回 0）
pub fn cleanup_old_logs(days: u32) -> usize {
    if days == 0 {
        return 0;
    }
    let dirs = crate::dirs::load_dirs();
    let max_age = std::time::Duration::from_secs(days as u64 * 86400);
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
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|age| age >= max_age)
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
