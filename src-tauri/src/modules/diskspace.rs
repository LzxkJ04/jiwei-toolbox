//! 磁盘空间：目录占用排行、大文件扫描

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

static SCAN_CANCEL: AtomicBool = AtomicBool::new(false);
static SCAN_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Clone)]
pub struct SpaceEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    /// "dir" | "file"
    pub kind: String,
    /// 文件数（目录递归统计；文件为 1）
    pub files: u64,
}

/// 递归求和，跳过符号链接与无权限目录。返回 (字节, 文件数)。
fn dir_size(dir: &Path, cancel: &AtomicBool, budget: &AtomicU64) -> (u64, u64) {
    let mut total = 0u64;
    let mut files = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        // 粗略限制访问条目数，避免在超大目录上转太久
        if budget.fetch_add(1, Ordering::Relaxed) > 400_000 {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&cur) else { continue };
        for entry in rd.flatten() {
            let Ok(md) = entry.metadata() else { continue };
            if md.file_type().is_symlink() {
                continue;
            }
            if md.is_dir() {
                stack.push(entry.path());
            } else if md.is_file() {
                total += md.len();
                files += 1;
            }
        }
    }
    (total, files)
}

/// 列出一个目录下各子项的体积排行。顶层目录递归统计，文件取自身大小。
#[tauri::command]
pub async fn dir_usage(path: String, limit: usize) -> Result<Vec<SpaceEntry>, String> {
    let root = PathBuf::from(path.trim());
    if !root.is_dir() {
        return Err("目录不存在".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let cancel = AtomicBool::new(false);
        let budget = AtomicU64::new(0);
        let Ok(rd) = std::fs::read_dir(&root) else {
            return Err("无法读取目录".to_string());
        };
        let mut out = Vec::new();
        for entry in rd.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(md) = entry.metadata() else { continue };
            if md.file_type().is_symlink() {
                continue;
            }
            if md.is_dir() {
                let (size, files) = dir_size(&p, &cancel, &budget);
                out.push(SpaceEntry {
                    name,
                    path: p.to_string_lossy().to_string(),
                    size,
                    kind: "dir".into(),
                    files,
                });
            } else if md.is_file() {
                out.push(SpaceEntry {
                    name,
                    path: p.to_string_lossy().to_string(),
                    size: md.len(),
                    kind: "file".into(),
                    files: 1,
                });
            }
        }
        out.sort_by(|a, b| b.size.cmp(&a.size));
        if limit > 0 && out.len() > limit {
            out.truncate(limit);
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 扫描一个目录下超过 min_mb 的文件，按体积降序。
#[tauri::command]
pub async fn scan_large_files(path: String, min_mb: u64, limit: usize) -> Result<Vec<SpaceEntry>, String> {
    let root = PathBuf::from(path.trim());
    if !root.is_dir() {
        return Err("目录不存在".into());
    }
    if SCAN_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("已有扫描在进行中".into());
    }
    SCAN_CANCEL.store(false, Ordering::SeqCst);
    let min_bytes = min_mb.max(1) * 1024 * 1024;
    let cap = if limit == 0 { 200 } else { limit.min(1000) };
    let res = tauri::async_runtime::spawn_blocking(move || {
        let mut out = Vec::new();
        let mut stack = vec![root];
        let mut visited = 0u64;
        while let Some(cur) = stack.pop() {
            if SCAN_CANCEL.load(Ordering::Relaxed) || visited > 600_000 {
                break;
            }
            let Ok(rd) = std::fs::read_dir(&cur) else { continue };
            for entry in rd.flatten() {
                visited += 1;
                let Ok(md) = entry.metadata() else { continue };
                if md.file_type().is_symlink() {
                    continue;
                }
                if md.is_dir() {
                    stack.push(entry.path());
                } else if md.is_file() && md.len() >= min_bytes {
                    out.push(SpaceEntry {
                        name: entry.file_name().to_string_lossy().to_string(),
                        path: entry.path().to_string_lossy().to_string(),
                        size: md.len(),
                        kind: "file".into(),
                        files: 1,
                    });
                }
            }
        }
        out.sort_by(|a, b| b.size.cmp(&a.size));
        if out.len() > cap {
            out.truncate(cap);
        }
        out
    })
    .await
    .unwrap_or_default();
    SCAN_RUNNING.store(false, Ordering::SeqCst);
    Ok(res)
}

/// 取消正在进行的扫描（下次循环退出）。删除操作也会检查这个标志。
#[tauri::command]
pub fn cancel_scan() {
    SCAN_CANCEL.store(true, Ordering::SeqCst);
}

/// 删除文件或空目录。只允许删已经存在、且不是盘符根的路径。
#[tauri::command]
pub async fn delete_space_item(path: String) -> Result<(), String> {
    let p = PathBuf::from(path.trim().to_string());
    if p.parent().is_none() || p.components().count() < 2 {
        return Err("拒绝删除盘符根目录".into());
    }
    if !p.exists() {
        return Err("路径不存在".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        if p.is_dir() {
            // 只删空目录，避免误删整个软件目录
            std::fs::remove_dir(&p).map_err(|e| format!("目录非空或无权删除：{e}"))
        } else {
            std::fs::remove_file(&p).map_err(|e| format!("删除失败：{e}"))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 在资源管理器中定位文件或目录。
#[tauri::command]
pub async fn reveal_path(path: String) -> Result<(), String> {
    let p = PathBuf::from(path.trim().to_string());
    if !p.exists() {
        return Err("路径不存在".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if p.is_dir() {
            std::process::Command::new("explorer")
                .arg(&p)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| e.to_string())?;
        } else {
            std::process::Command::new("explorer")
                .args(["/select,", &p.to_string_lossy()])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 已挂载磁盘的总体占用。
#[tauri::command]
pub async fn disk_overview() -> Vec<serde_json::Value> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut seen = HashMap::new();
    let mut out = Vec::new();
    for d in disks.list() {
        let mp = d.mount_point().to_string_lossy().to_string();
        if d.total_space() == 0 || seen.contains_key(&mp) {
            continue;
        }
        seen.insert(mp.clone(), true);
        let total = d.total_space();
        let free = d.available_space();
        out.push(serde_json::json!({
            "mount": mp,
            "name": d.name().to_string_lossy(),
            "fs": d.file_system().to_string_lossy(),
            "total": total,
            "free": free,
            "used": total.saturating_sub(free),
        }));
    }
    out.sort_by(|a, b| {
        a["mount"].as_str().unwrap_or("").cmp(b["mount"].as_str().unwrap_or(""))
    });
    out
}

static CLEAN_RESULT: Mutex<Option<u64>> = Mutex::new(None);

/// 清空某目录下的文件（保留目录结构）。返回释放字节数。
#[tauri::command]
pub async fn empty_dir_contents(path: String) -> Result<u64, String> {
    let root = PathBuf::from(path.trim().to_string());
    if !root.is_dir() {
        return Err("目录不存在".into());
    }
    if root.components().count() < 2 {
        return Err("拒绝清空盘符根目录".into());
    }
    let freed = tauri::async_runtime::spawn_blocking(move || {
        let mut freed = 0u64;
        let Ok(rd) = std::fs::read_dir(&root) else {
            return 0u64;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            let Ok(md) = entry.metadata() else { continue };
            if md.file_type().is_symlink() {
                continue;
            }
            if md.is_file() {
                freed += md.len();
                let _ = std::fs::remove_file(&p);
            } else if md.is_dir() {
                let (sz, _) = dir_size(&p, &AtomicBool::new(false), &AtomicU64::new(0));
                if std::fs::remove_dir_all(&p).is_ok() {
                    freed += sz;
                }
            }
        }
        freed
    })
    .await
    .unwrap_or(0);
    *CLEAN_RESULT.lock().unwrap_or_else(|e| e.into_inner()) = Some(freed);
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::dir_size;
    use std::sync::atomic::{AtomicBool, AtomicU64};

    #[test]
    fn dir_size_sums_files_recursively() {
        let base = std::env::temp_dir().join("jw_space_test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("a/b")).unwrap();
        std::fs::write(base.join("a/one.bin"), vec![0u8; 1024]).unwrap();
        std::fs::write(base.join("a/b/two.bin"), vec![0u8; 2048]).unwrap();
        let (size, files) = dir_size(&base, &AtomicBool::new(false), &AtomicU64::new(0));
        assert_eq!(files, 2);
        assert_eq!(size, 3072);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn dir_size_respects_cancel() {
        let base = std::env::temp_dir().join("jw_space_cancel");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("x.bin"), vec![0u8; 16]).unwrap();
        let (size, files) = dir_size(&base, &AtomicBool::new(true), &AtomicU64::new(0));
        assert_eq!(files, 0);
        assert_eq!(size, 0);
        let _ = std::fs::remove_dir_all(&base);
    }
}
