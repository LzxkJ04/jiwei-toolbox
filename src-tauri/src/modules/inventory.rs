//! 软件清单备份：交互式清单保存/加载/管理（写入 data\备份导出\*.json）
//! 与 backup.rs 的目录 zip 备份互不影响；文件名做路径穿越防护。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Emitter;

use super::dirs::load_dirs;

#[derive(Serialize, Deserialize, Clone)]
pub struct InventoryApp {
    pub name: String,
    pub version: String,
    pub publisher: String,
    #[serde(default)]
    pub install_location: String,
}

#[derive(Serialize, Deserialize)]
pub struct InventoryDoc {
    pub kind: String, // "jiwei-inventory"
    pub name: String,
    pub created: String,
    pub apps: Vec<InventoryApp>,
}

/// 备份导出目录（不存在则创建）
fn backup_dir() -> Result<PathBuf, String> {
    let d = load_dirs();
    let dir = PathBuf::from(&d.backup);
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建备份目录: {}", e))?;
    Ok(dir)
}

/// 校验清单文件名：拒绝路径分隔符 / .. / 非法字符
fn validate_inventory_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() || n.len() > 80 {
        return Err("备份名称为空或过长（≤80 字符）".into());
    }
    if n.contains("..")
        || n.contains('/')
        || n.contains('\\')
        || n.contains(':')
        || n.contains('<')
        || n.contains('>')
        || n.contains('|')
        || n.contains('?')
        || n.contains('*')
        || n.contains('"')
    {
        return Err("备份名称含非法字符".into());
    }
    Ok(())
}

fn inventory_path(name: &str) -> Result<PathBuf, String> {
    validate_inventory_name(name)?;
    Ok(backup_dir()?.join(format!("{}.jwinv.json", name.trim())))
}

/// 保存清单（同名覆盖）
#[tauri::command]
pub fn save_inventory(
    app: tauri::AppHandle,
    name: String,
    apps: Vec<InventoryApp>,
) -> Result<serde_json::Value, String> {
    let path = inventory_path(&name)?;
    let doc = InventoryDoc {
        kind: "jiwei-inventory".into(),
        name: name.trim().to_string(),
        created: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        apps,
    };
    let json = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("写入失败: {}", e))?;
    super::logs::op_log(&load_dirs(), "inventory", "save", &path.to_string_lossy());
    let _ = app.emit(
        "op-logged",
        serde_json::json!({"module":"inventory","action":"save"}),
    );
    Ok(serde_json::json!({"file": path.to_string_lossy(), "count": doc.apps.len()}))
}

#[derive(Serialize)]
pub struct InventoryMeta {
    pub name: String,
    pub file: String,
    pub created: String,
    pub count: usize,
    pub size: u64,
    pub mtime: u64,
}

/// 列出全部清单
#[tauri::command]
pub fn list_inventories() -> Vec<InventoryMeta> {
    let Ok(dir) = backup_dir() else {
        return vec![];
    };
    let mut out = vec![];
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "json").unwrap_or(true) {
                continue;
            }
            let fname = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(stem) = fname.strip_suffix(".jwinv.json") else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(doc) = serde_json::from_str::<InventoryDoc>(&text) else {
                continue;
            };
            let md = e.metadata().ok();
            out.push(InventoryMeta {
                name: doc.name,
                file: p.to_string_lossy().into_owned(),
                created: doc.created,
                count: doc.apps.len(),
                size: md.as_ref().map(|m| m.len()).unwrap_or(0),
                mtime: md
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            });
        }
    }
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    out
}

/// 读取单个清单内容
#[tauri::command]
pub fn load_inventory(name: String) -> Result<InventoryDoc, String> {
    let path = inventory_path(&name)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {}", e))?;
    serde_json::from_str(&text).map_err(|e| format!("清单文件损坏: {}", e))
}

/// 删除清单
#[tauri::command]
pub fn delete_inventory(name: String) -> Result<(), String> {
    let path = inventory_path(&name)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("删除失败: {}", e))?;
        super::logs::op_log(&load_dirs(), "inventory", "delete", name.trim());
    }
    Ok(())
}

/// 重命名清单
#[tauri::command]
pub fn rename_inventory(old_name: String, new_name: String) -> Result<(), String> {
    let old = inventory_path(&old_name)?;
    let new = inventory_path(&new_name)?;
    if !old.exists() {
        return Err("原清单不存在".into());
    }
    if new.exists() {
        return Err("目标名称已存在".into());
    }
    std::fs::rename(&old, &new).map_err(|e| format!("重命名失败: {}", e))?;
    super::logs::op_log(&load_dirs(), "inventory", "rename", &format!("{} -> {}", old_name.trim(), new_name.trim()));
    Ok(())
}

/// 用系统默认方式打开路径（文件用关联程序，目录用资源管理器）
#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    let p = path.trim();
    if p.is_empty() || p.contains("..") {
        return Err("非法路径".into());
    }
    let target = PathBuf::from(p);
    if !target.exists() {
        return Err(format!("路径不存在: {}", p));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if target.is_dir() {
            std::process::Command::new("explorer.exe")
                .arg(p)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| e.to_string())?;
        } else {
            // ShellExecute 语义：用 cmd start 打开关联程序
            std::process::Command::new("cmd")
                .args(["/C", "start", "", p])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        super::logs::op_log(&load_dirs(), "common", "open_path", p);
    }
    #[cfg(not(windows))]
    {
        let _ = target;
        return Err("仅支持 Windows".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_valid_and_invalid() {
        assert!(validate_inventory_name("我的清单 2026").is_ok());
        assert!(validate_inventory_name("backup-1").is_ok());
        assert!(validate_inventory_name("").is_err());
        assert!(validate_inventory_name("..\\evil").is_err());
        assert!(validate_inventory_name("a/b").is_err());
        assert!(validate_inventory_name("a\\b").is_err());
        assert!(validate_inventory_name("a:b").is_err());
        assert!(validate_inventory_name("a*b").is_err());
        assert!(validate_inventory_name(&"x".repeat(81)).is_err());
    }

    #[test]
    fn doc_roundtrip() {
        let doc = InventoryDoc {
            kind: "jiwei-inventory".into(),
            name: "测试".into(),
            created: "2026-10-18 02:00:00".into(),
            apps: vec![InventoryApp {
                name: "Chrome".into(),
                version: "120".into(),
                publisher: "Google".into(),
                install_location: "C:\\Program Files\\Chrome".into(),
            }],
        };
        let json = serde_json::to_string(&doc).unwrap();
        let back: InventoryDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(back.apps[0].name, "Chrome");
        assert_eq!(back.name, "测试");
    }

    #[test]
    fn doc_tolerates_missing_install_location() {
        let json = r#"{"kind":"jiwei-inventory","name":"n","created":"t","apps":[{"name":"a","version":"1","publisher":"p"}]}"#;
        let doc: InventoryDoc = serde_json::from_str(json).unwrap();
        assert_eq!(doc.apps[0].install_location, "");
    }
}
