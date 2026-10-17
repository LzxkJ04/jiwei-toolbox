//! 垃圾扫描清理与缓存统计

use serde::Serialize;
use std::path::{Path, PathBuf};

/// 递归统计目录大小（错误静默跳过）
pub(crate) fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    for e in entries.flatten() {
        let p = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() => total += dir_size(&p),
            Ok(t) if t.is_file() => {
                total += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
            _ => {}
        }
    }
    total
}

/// 清空目录内容（不删目录本身）
fn clear_dir(path: &Path) {
    if !path.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(path) {
        for e in entries.flatten() {
            let p = e.path();
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        }
    }
}

#[derive(Serialize)]
pub struct JunkItem {
    pub path: String,
    pub size: u64,
    /// "temp" | "recycle" | "thumbcache" | "browser"
    pub kind: String,
}

/// 扫描垃圾：%TEMP%/*、回收站、缩略图缓存、Chrome/Edge 缓存（存在才列）
#[tauri::command]
pub fn scan_junk() -> Vec<JunkItem> {
    let mut out = Vec::new();

    // %TEMP% 顶层各项
    if let Ok(temp) = std::env::var("TEMP") {
        if let Ok(entries) = std::fs::read_dir(&temp) {
            for e in entries.flatten() {
                let p = e.path();
                let size = if p.is_dir() { dir_size(&p) } else { e.metadata().map(|m| m.len()).unwrap_or(0) };
                out.push(JunkItem { path: p.to_string_lossy().into_owned(), size, kind: "temp".into() });
            }
        }
    }

    // 回收站
    let recycle = Path::new(r"C:\$Recycle.Bin");
    if recycle.is_dir() {
        out.push(JunkItem {
            path: recycle.to_string_lossy().into_owned(),
            size: dir_size(recycle),
            kind: "recycle".into(),
        });
    }

    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        // 缩略图缓存（thumbcache_*.db / iconcache_*.db）
        let explorer = Path::new(&la).join(r"Microsoft\Windows\Explorer");
        if explorer.is_dir() {
            let mut size = 0u64;
            if let Ok(entries) = std::fs::read_dir(&explorer) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    if name.starts_with("thumbcache_") || name.starts_with("iconcache_") {
                        size += e.metadata().map(|m| m.len()).unwrap_or(0);
                    }
                }
            }
            if size > 0 {
                out.push(JunkItem { path: explorer.to_string_lossy().into_owned(), size, kind: "thumbcache".into() });
            }
        }

        // Chrome / Edge 缓存
        for sub in [r"Google\Chrome\User Data\Default\Cache", r"Microsoft\Edge\User Data\Default\Cache"] {
            let p = Path::new(&la).join(sub);
            if p.is_dir() {
                out.push(JunkItem { path: p.to_string_lossy().into_owned(), size: dir_size(&p), kind: "browser".into() });
            }
        }
    }
    out
}

/// 清理垃圾（删除失败即跳过：文件占用等）
#[tauri::command]
pub fn clean_junk(paths: Vec<String>) {
    let count = paths.len();
    let dirs = crate::dirs::load_dirs();
    for p in &paths {
        let pp = Path::new(p);
        let _ = if pp.is_dir() { std::fs::remove_dir_all(pp) } else { std::fs::remove_file(pp) };
    }
    crate::logs::op_log(&dirs, "cache", "clean_junk", &format!("清理 {} 个垃圾项", count));
}

/// 应用缓存统计（downloads / logs / temp）
#[tauri::command]
pub fn cache_stats() -> serde_json::Value {
    let dirs = crate::dirs::load_dirs();
    let downloads = Path::new(&dirs.cache).join("downloads");
    let temp = Path::new(&dirs.cache).join("temp");
    let d = dir_size(&downloads);
    let l = dir_size(Path::new(&dirs.log));
    let t = dir_size(&temp);
    serde_json::json!({
        "dirs": [
            { "name": "downloads", "size": d },
            { "name": "logs", "size": l },
            { "name": "temp", "size": t },
        ],
        "total": d + l + t,
    })
}

/// 清理应用缓存（downloads / temp / logs 内容）
#[tauri::command]
pub fn clean_cache() {
    let dirs = crate::dirs::load_dirs();
    clear_dir(&Path::new(&dirs.cache).join("downloads"));
    clear_dir(&Path::new(&dirs.cache).join("temp"));
    clear_dir(Path::new(&dirs.log));
}

// ---------------------------------------------------------------------------
// 缓存精细管理：文件明细 + 按文件名精确删除（防路径穿越）
// ---------------------------------------------------------------------------

/// 缓存文件明细项（camelCase 序列化）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheFile {
    pub name: String,
    pub path: String,
    pub size: u64,
    /// 修改时间（Unix 秒）
    pub mtime: i64,
    /// "download" | "temp" | "log" | "other"
    pub kind: String,
}

/// 把 dir 下的一级文件追加到 out（子目录跳过，错误静默）
fn push_dir_files(out: &mut Vec<CacheFile>, dir: &Path, kind: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        out.push(CacheFile {
            name: e.file_name().to_string_lossy().into_owned(),
            path: p.to_string_lossy().into_owned(),
            size: md.len(),
            mtime,
            kind: kind.into(),
        });
    }
}

/// 缓存文件明细：cache\downloads（download）、cache\temp（temp）、
/// 日志目录（log）、cache 根一级文件（other，如 softwares.json）
#[tauri::command]
pub fn cache_files() -> Vec<CacheFile> {
    let dirs = crate::dirs::load_dirs();
    let cache = Path::new(&dirs.cache);
    let mut out = Vec::new();
    push_dir_files(&mut out, &cache.join("downloads"), "download");
    push_dir_files(&mut out, &cache.join("temp"), "temp");
    push_dir_files(&mut out, Path::new(&dirs.log), "log");
    push_dir_files(&mut out, cache, "other");
    out
}

/// 按文件名精确删除缓存文件：names 必须是纯文件名（禁止路径分隔符与 ".."），
/// 只在 cache\downloads、cache\temp、日志目录、cache 根内匹配删除。
/// 返回实际删除的文件数。
#[tauri::command]
pub fn cache_delete(names: Vec<String>) -> Result<usize, String> {
    for n in &names {
        if n.trim().is_empty() || n.contains('\\') || n.contains('/') || n.contains("..") {
            return Err(format!("非法文件名: {}", n));
        }
    }
    let dirs = crate::dirs::load_dirs();
    let cache = Path::new(&dirs.cache);
    let search_dirs = [
        cache.join("downloads"),
        cache.join("temp"),
        PathBuf::from(&dirs.log),
        cache.to_path_buf(),
    ];
    let mut deleted = 0usize;
    for n in &names {
        for d in &search_dirs {
            let p = d.join(n);
            // 只删普通文件（join 后仍是目录内直接子项，无穿越风险）
            if p.is_file() && std::fs::remove_file(&p).is_ok() {
                deleted += 1;
            }
        }
    }
    crate::logs::op_log(&dirs, "cache", "cache_delete", &format!("精细删除 {} 个缓存文件", deleted));
    Ok(deleted)
}
