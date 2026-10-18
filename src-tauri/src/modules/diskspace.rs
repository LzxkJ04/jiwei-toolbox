//! 磁盘空间：目录占用排行、大文件扫描、重复文件检测

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

// ---------- 重复文件 ----------

/// 一组内容完全相同的文件
#[derive(Serialize, Clone)]
pub struct DuplicateGroup {
    /// 内容哈希（前 16 位十六进制，仅供前端去重展示）
    pub hash: String,
    pub size: u64,
    /// 所有副本的完整路径（至少 2 个）
    pub paths: Vec<String>,
    /// 除保留份以外可回收的字节数
    pub wasted: u64,
}

/// 把「同尺寸候选」按内容分组。纯函数，便于单测。
/// candidates: (路径, 尺寸)，已按尺寸分好组的同一尺寸集合内调用。
fn group_duplicates(candidates: &[(String, u64)]) -> Vec<DuplicateGroup> {
    use std::collections::HashMap;
    let mut by_hash: HashMap<String, Vec<String>> = HashMap::new();
    for (path, _) in candidates {
        // 读不出内容就跳过，不把它当成重复
        let Ok(bytes) = std::fs::read(path) else { continue };
        let hash = format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(&bytes));
        by_hash.entry(hash[..16].to_string()).or_default().push(path.clone());
    }
    let mut out: Vec<DuplicateGroup> = by_hash
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(hash, paths)| {
            let size = candidates
                .iter()
                .find(|(p, _)| p == &paths[0])
                .map(|(_, s)| *s)
                .unwrap_or(0);
            let wasted = size * (paths.len() as u64 - 1);
            DuplicateGroup { hash, size, paths, wasted }
        })
        .collect();
    // 可回收空间大的排前面
    out.sort_by(|a, b| b.wasted.cmp(&a.wasted));
    out
}

static DUP_RUNNING: AtomicBool = AtomicBool::new(false);

/// 扫描重复文件的同步实现：先按大小分桶，只对同尺寸文件算 SHA-256。
fn collect_duplicates(root: PathBuf, min_bytes: u64) -> Vec<DuplicateGroup> {
    let mut by_size: HashMap<u64, Vec<String>> = HashMap::new();
    let mut stack = vec![root];
    let mut visited = 0u64;
    while let Some(cur) = stack.pop() {
        if SCAN_CANCEL.load(Ordering::Relaxed) || visited > 400_000 {
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
                by_size
                    .entry(md.len())
                    .or_default()
                    .push(entry.path().to_string_lossy().to_string());
            }
        }
    }
    // 只有同尺寸且至少 2 个的桶才值得算哈希
    let mut out = Vec::new();
    for (size, paths) in by_size {
        if paths.len() < 2 {
            continue;
        }
        let cand: Vec<(String, u64)> = paths.into_iter().map(|p| (p, size)).collect();
        out.extend(group_duplicates(&cand));
    }
    out.sort_by(|a, b| b.wasted.cmp(&a.wasted));
    // 限制返回条数，避免一次渲染上千行
    out.truncate(300);
    out
}

/// 扫描重复文件。
/// min_kb 用来跳过大量小文件（默认 1KB 以上）。
#[tauri::command]
pub async fn scan_duplicates(path: String, min_kb: u64) -> Result<Vec<DuplicateGroup>, String> {
    let root = PathBuf::from(path.trim());
    if !root.is_dir() {
        return Err("目录不存在".into());
    }
    if DUP_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("已有重复扫描在进行中".into());
    }
    SCAN_CANCEL.store(false, Ordering::SeqCst);
    let min_bytes = min_kb.max(1) * 1024;
    let res = tauri::async_runtime::spawn_blocking(move || collect_duplicates(root, min_bytes))
        .await
        .unwrap_or_default();
    DUP_RUNNING.store(false, Ordering::SeqCst);
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::{collect_duplicates, dir_size, group_duplicates};
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

    #[test]
    fn duplicate_groups_need_identical_bytes() {
        let base = std::env::temp_dir().join("jw_dup_test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.txt"), b"same-content").unwrap();
        std::fs::write(base.join("b.txt"), b"same-content").unwrap();
        // 同长度但内容不同，不能被判成重复
        std::fs::write(base.join("c.txt"), b"diff-content").unwrap();

        let cand: Vec<(String, u64)> = ["a.txt", "b.txt", "c.txt"]
            .iter()
            .map(|n| {
                let p = base.join(n);
                let len = std::fs::metadata(&p).unwrap().len();
                (p.to_string_lossy().to_string(), len)
            })
            .collect();
        let groups = group_duplicates(&cand);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].paths.len(), 2);
        assert_eq!(groups[0].size, 12);
        assert_eq!(groups[0].wasted, 12);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn single_file_is_not_a_group() {
        let base = std::env::temp_dir().join("jw_dup_single");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let p = base.join("only.txt");
        std::fs::write(&p, b"unique").unwrap();
        let cand = vec![(p.to_string_lossy().to_string(), 6u64)];
        assert!(group_duplicates(&cand).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 真实目录端到端：同内容归一组、同长度不同内容不误判、小于阈值跳过
    #[test]
    fn collect_duplicates_walks_real_dirs() {
        let base = std::env::temp_dir().join("jw_dup_walk");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("a/nested")).unwrap();
        std::fs::create_dir_all(base.join("b")).unwrap();
        let payload = vec![7u8; 4096];
        std::fs::write(base.join("a/one.bin"), &payload).unwrap();
        std::fs::write(base.join("a/nested/two.bin"), &payload).unwrap();
        std::fs::write(base.join("b/three.bin"), &payload).unwrap();
        // 同长度不同内容
        let mut other = payload.clone();
        other[0] = 9;
        std::fs::write(base.join("b/other.bin"), &other).unwrap();
        // 小于阈值，应被跳过
        std::fs::write(base.join("b/tiny.bin"), b"xy").unwrap();

        let groups = collect_duplicates(base.clone(), 1024);
        assert_eq!(groups.len(), 1, "应该只有一组真重复");
        assert_eq!(groups[0].paths.len(), 3);
        assert_eq!(groups[0].size, 4096);
        assert_eq!(groups[0].wasted, 8192);
        let _ = std::fs::remove_dir_all(&base);
    }
}
