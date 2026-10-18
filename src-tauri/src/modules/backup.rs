//! 配置备份与恢复（.jwbak = zip 压缩包）

use serde::Serialize;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct BackupInfo {
    pub file: String,
    pub size: u64,
    pub time: String,
}

fn add_dir_to_zip(zw: &mut zip::ZipWriter<File>, dir: &Path, base: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    for e in entries.flatten() {
        let p = e.path();
        let rel = p.strip_prefix(base).map_err(|e| e.to_string())?;
        let name = rel.to_string_lossy().replace('\\', "/");
        if p.is_dir() {
            add_dir_to_zip(zw, &p, base)?;
        } else if p.is_file() {
            zw.start_file(&name, zip::write::SimpleFileOptions::default())
                .map_err(|e| format!("{}: {}", name, e))?;
            let mut f = File::open(&p).map_err(|e| format!("{}: {}", p.display(), e))?;
            std::io::copy(&mut f, zw).map_err(|e| format!("{}: {}", p.display(), e))?;
        }
    }
    Ok(())
}

/// 创建配置备份：打包 config 目录 → backup\backup-YYYYMMDD-HHMMSS.jwbak
#[tauri::command]
pub async fn create_backup(note: String) -> Result<serde_json::Value, String> {
    let dirs = crate::dirs::load_dirs();
    let src = PathBuf::from(&dirs.config);
    if !src.is_dir() {
        return Err(format!("配置目录不存在: {}", dirs.config));
    }
    let backup_dir = PathBuf::from(&dirs.backup);
    std::fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;

    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = backup_dir.join(format!("backup-{}.jwbak", ts));
    let file = File::create(&dest).map_err(|e| e.to_string())?;
    let mut zw = zip::ZipWriter::new(file);

    // 备注元信息
    zw.start_file("_meta.json", zip::write::SimpleFileOptions::default())
        .map_err(|e| e.to_string())?;
    let meta = serde_json::json!({
        "note": note,
        "time": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    });
    zw.write_all(serde_json::to_string(&meta).map_err(|e| e.to_string())?.as_bytes())
        .map_err(|e| e.to_string())?;

    add_dir_to_zip(&mut zw, &src, &src)?;
    zw.finish().map_err(|e| e.to_string())?;
    crate::logs::op_log(&dirs, "backup", "create_backup", &format!("创建备份 {}（备注: {}）", dest.to_string_lossy(), note));
    Ok(serde_json::json!({ "file": dest.to_string_lossy() }))
}

/// 列出全部备份
#[tauri::command]
pub async fn list_backups() -> Vec<BackupInfo> {
    let dirs = crate::dirs::load_dirs();
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dirs.backup) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.extension().map(|x| x.eq_ignore_ascii_case("jwbak")).unwrap_or(false) {
                continue;
            }
            if let Ok(meta) = e.metadata() {
                let time = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
                    .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_default();
                out.push(BackupInfo { file: p.to_string_lossy().into_owned(), size: meta.len(), time });
            }
        }
    }
    out.sort_by(|a, b| b.file.cmp(&a.file));
    out
}

/// 恢复备份：解压覆盖 config 目录（含路径穿越防护）
#[tauri::command]
pub fn restore_backup(file: String) -> Result<(), String> {
    let dirs = crate::dirs::load_dirs();
    let zf = File::open(&file).map_err(|e| format!("{}: {}", file, e))?;
    let mut archive = zip::ZipArchive::new(zf).map_err(|e| e.to_string())?;
    let cfg = PathBuf::from(&dirs.config);
    std::fs::create_dir_all(&cfg).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else { continue };
        if rel == Path::new("_meta.json") {
            continue;
        }
        if entry.name().ends_with('/') {
            std::fs::create_dir_all(cfg.join(&rel)).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = cfg.join(&rel).parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = File::create(cfg.join(&rel)).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }
    crate::logs::op_log(&dirs, "backup", "restore_backup", &format!("恢复备份 {}", file));
    Ok(())
}

/// 递归复制目录（用于导入前备份现有配置）
fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {}", dest.display(), e))?;
    for e in std::fs::read_dir(src).map_err(|e| format!("{}: {}", src.display(), e))?.flatten() {
        let p = e.path();
        let target = dest.join(p.file_name().unwrap_or_default());
        if p.is_dir() {
            copy_dir(&p, &target)?;
        } else if p.is_file() {
            std::fs::copy(&p, &target).map_err(|e| format!("{}: {}", p.display(), e))?;
        }
    }
    Ok(())
}

/// 导出配置：把配置目录打包 zip（config-export-YYYYMMDD-HHMMSS.zip）到备份导出目录；
/// target 可指定导出目标目录（缺省用备份目录）。返回 {file}
#[tauri::command]
pub fn export_config(target: Option<String>) -> Result<serde_json::Value, String> {
    let dirs = crate::dirs::load_dirs();
    let src = PathBuf::from(&dirs.config);
    if !src.is_dir() {
        return Err(format!("配置目录不存在: {}", dirs.config));
    }
    let dest_dir = match target.as_deref() {
        Some(t) if !t.trim().is_empty() => PathBuf::from(t),
        _ => PathBuf::from(&dirs.backup),
    };
    std::fs::create_dir_all(&dest_dir).map_err(|e| format!("{}: {}", dest_dir.display(), e))?;

    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = dest_dir.join(format!("config-export-{}.zip", ts));
    let file = File::create(&dest).map_err(|e| e.to_string())?;
    let mut zw = zip::ZipWriter::new(file);
    add_dir_to_zip(&mut zw, &src, &src)?;
    zw.finish().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "file": dest.to_string_lossy() }))
}

/// 导入配置：解压 zip 覆盖配置目录；覆盖前先把现有配置备份到
/// 备份目录下 config-import-bak-YYYYMMDD-HHMMSS
#[tauri::command]
pub fn import_config(file: String) -> Result<(), String> {
    let dirs = crate::dirs::load_dirs();
    let zf = File::open(&file).map_err(|e| format!("{}: {}", file, e))?;
    let mut archive = zip::ZipArchive::new(zf).map_err(|e| e.to_string())?;
    let cfg = PathBuf::from(&dirs.config);
    std::fs::create_dir_all(&cfg).map_err(|e| e.to_string())?;

    // 覆盖前备份现有配置
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let bak = PathBuf::from(&dirs.backup).join(format!("config-import-bak-{}", ts));
    copy_dir(&cfg, &bak).map_err(|e| format!("备份现有配置失败: {}", e))?;

    // 解压覆盖（含路径穿越防护）
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else { continue };
        if entry.name().ends_with('/') {
            std::fs::create_dir_all(cfg.join(&rel)).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = cfg.join(&rel).parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = File::create(cfg.join(&rel)).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 开机自动备份软件清单（settings.json auto_backup_inventory: bool）
// ---------------------------------------------------------------------------

/// 读取"开机自动备份清单"开关（默认关闭）
#[tauri::command]
pub fn get_auto_backup_inventory() -> bool {
    crate::common::read_setting_raw("auto_backup_inventory")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 设置"开机自动备份清单"开关
#[tauri::command]
pub fn set_auto_backup_inventory(enable: bool) -> Result<(), String> {
    crate::common::write_setting_raw("auto_backup_inventory", serde_json::Value::Bool(enable))
}

/// 枚举已安装软件并写入 <data>\备份导出\software-inventory-auto-YYYYMMDD.json
/// （UTF-8 JSON 数组：[{name, version, publisher, install_location}]）+ op_log
fn write_inventory_snapshot(dirs: &crate::dirs::Dirs) -> Result<String, String> {
    let apps = crate::uninstall::list_installed_inner();
    let items: Vec<serde_json::Value> = apps
        .iter()
        .map(|a| {
            serde_json::json!({
                "name": a.name,
                "version": a.version,
                "publisher": a.publisher,
                "install_location": a.install_location,
            })
        })
        .collect();
    let backup_dir = PathBuf::from(&dirs.backup);
    std::fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;
    let dest = backup_dir.join(format!(
        "software-inventory-auto-{}.json",
        chrono::Local::now().format("%Y%m%d")
    ));
    let text = serde_json::to_string_pretty(&items).map_err(|e| e.to_string())?;
    std::fs::write(&dest, text).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().into_owned())
}

/// 启动时后台自动备份软件清单（lib.rs setup 主模式下调用）：
/// 开关开启时延迟 5 秒等待启动高峰过后写清单快照
pub fn startup_auto_inventory() {
    std::thread::spawn(|| {
        if !get_auto_backup_inventory() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
        let dirs = crate::dirs::load_dirs();
        match write_inventory_snapshot(&dirs) {
            Ok(path) => crate::logs::op_log(&dirs, "backup", "auto_inventory", &format!("开机自动备份软件清单 -> {}", path)),
            Err(e) => crate::logs::op_log(&dirs, "backup", "auto_inventory", &format!("开机自动备份软件清单失败: {}", e)),
        }
    });
}
