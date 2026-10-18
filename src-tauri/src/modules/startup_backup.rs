//! 启动项备份与还原
//!
//! 原实现「禁用启动项」直接把 Run 值删掉，命令只存在内存缓存里，
//! 程序一关就永久丢失，启动文件夹的 .lnk 更是直接报错不支持。
//! 这里改成把禁用项落盘到 配置\启动项备份.json，随时可原样还原。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StartupBackup {
    /// 启动项名称（Run 值名或 .lnk 文件名，不含扩展名）
    pub name: String,
    /// 原始命令 / .lnk 完整路径
    pub command: String,
    /// "hkcu-run" | "hklm-run" | "hklm-run-wow" | "startup-folder-user" | "startup-folder-all"
    pub source: String,
    /// 禁用时间（Unix 秒）
    pub disabled_at: u64,
}

fn backup_file() -> PathBuf {
    PathBuf::from(crate::dirs::load_dirs().config).join("启动项备份.json")
}

/// 启动文件夹里的 .lnk 被移到这里，原位置只留记录
pub fn shortcut_store_dir() -> PathBuf {
    PathBuf::from(crate::dirs::load_dirs().config).join("启动项备份文件")
}

/// 把启动文件夹里的快捷方式移到备份目录，返回目标路径。
pub fn stash_shortcut(lnk: &std::path::Path) -> Result<PathBuf, String> {
    let store = shortcut_store_dir();
    std::fs::create_dir_all(&store).map_err(|e| format!("无法创建备份目录：{e}"))?;
    let fname = lnk
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| "快捷方式路径异常".to_string())?;
    let dest = store.join(&fname);
    std::fs::rename(lnk, &dest).map_err(|e| format!("移动快捷方式失败：{e}"))?;
    Ok(dest)
}

/// 把之前移走的快捷方式放回原位。
pub fn unstash_shortcut(stashed: &std::path::Path, original_dir: &std::path::Path) -> Result<PathBuf, String> {
    if !stashed.exists() {
        return Err("备份的快捷方式已不存在".into());
    }
    std::fs::create_dir_all(original_dir).map_err(|e| e.to_string())?;
    let fname = stashed
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| "备份文件名异常".to_string())?;
    let dest = original_dir.join(&fname);
    std::fs::rename(stashed, &dest).map_err(|e| format!("还原快捷方式失败：{e}"))?;
    Ok(dest)
}

/// 读取全部已禁用备份。文件损坏时返回空表，不阻塞界面。
pub fn load_backups() -> Vec<StartupBackup> {
    let Ok(bytes) = std::fs::read(backup_file()) else {
        return vec![];
    };
    let text = crate::common::strip_bom(String::from_utf8_lossy(&bytes).into_owned());
    serde_json::from_str::<Vec<StartupBackup>>(&text).unwrap_or_default()
}

fn save_backups(list: &[StartupBackup]) -> Result<(), String> {
    let path = backup_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// 记录一条禁用备份；同名同来源的旧记录会被替换，避免重复堆积。
pub fn remember(name: &str, command: &str, source: &str) -> Result<(), String> {
    let mut list = load_backups();
    list.retain(|b| !(b.name == name && b.source == source));
    list.push(StartupBackup {
        name: name.to_string(),
        command: command.to_string(),
        source: source.to_string(),
        disabled_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    save_backups(&list)
}

/// 按名称取出备份（不知道来源时用；Run 键与快捷方式同名时优先 Run 项）。
pub fn take_by_name(name: &str) -> Option<StartupBackup> {
    let mut list = load_backups();
    let idx = list
        .iter()
        .position(|b| b.name == name && b.source.contains("run"))
        .or_else(|| list.iter().position(|b| b.name == name))?;
    let item = list.remove(idx);
    let _ = save_backups(&list);
    Some(item)
}

/// 把 "HKCU\Software\...\Run" 这类定位串映射成来源标识。
pub fn source_of_location(location: &str) -> &'static str {
    let l = location.to_ascii_lowercase();
    if l.contains("wow6432node") {
        "hklm-run-wow"
    } else if l.starts_with("hkcu") {
        "hkcu-run"
    } else if l.starts_with("hklm") {
        "hklm-run"
    } else if l.contains("appdata") || l.contains(r"\users\") {
        "startup-folder-user"
    } else {
        "startup-folder-all"
    }
}

#[cfg(test)]
mod tests {
    use super::{source_of_location, StartupBackup};

    #[test]
    fn location_maps_to_source() {
        assert_eq!(source_of_location(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"), "hkcu-run");
        assert_eq!(
            source_of_location(r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run"),
            "hklm-run-wow"
        );
        assert_eq!(source_of_location(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run"), "hklm-run");
        assert_eq!(
            source_of_location(r"C:\Users\Administrator\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup"),
            "startup-folder-user"
        );
    }

    #[test]
    fn backup_roundtrip_json() {
        let b = StartupBackup {
            name: "微信".into(),
            command: r#""C:\Program Files\WeChat\WeChat.exe" -autorun"#.into(),
            source: "hkcu-run".into(),
            disabled_at: 1_700_000_000,
        };
        let text = serde_json::to_string(&b).unwrap();
        let back: StartupBackup = serde_json::from_str(&text).unwrap();
        assert_eq!(b, back);
        assert!(text.contains("hkcu-run"));
    }

    /// 快捷方式移走 → 放回，内容必须一字不差
    #[test]
    fn shortcut_stash_and_restore_roundtrip() {
        use super::{stash_shortcut, unstash_shortcut};
        let base = std::env::temp_dir().join("jw_lnk_roundtrip");
        let _ = std::fs::remove_dir_all(&base);
        let orig_dir = base.join("Startup");
        std::fs::create_dir_all(&orig_dir).unwrap();
        let lnk = orig_dir.join("Demo.lnk");
        std::fs::write(&lnk, b"fake-lnk-payload").unwrap();

        let stashed = stash_shortcut(&lnk).expect("应能移入备份目录");
        assert!(!lnk.exists(), "原位置应已不存在");
        assert!(stashed.exists(), "备份文件应存在");

        let restored = unstash_shortcut(&stashed, &orig_dir).expect("应能放回原位");
        assert!(restored.exists());
        assert_eq!(std::fs::read(&restored).unwrap(), b"fake-lnk-payload");
        assert!(!stashed.exists(), "备份文件应已移走");
        let _ = std::fs::remove_dir_all(&base);
    }
}
