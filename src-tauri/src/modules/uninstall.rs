//! 已安装软件枚举、残留扫描与清理

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use winreg::enums::*;
use winreg::HKEY;
use winreg::RegKey;

const UNINSTALL_PATH: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const WOW_UNINSTALL_PATH: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";

/// 已安装应用
#[derive(Serialize)]
pub struct InstalledApp {
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub uninstaller: String,
    pub key: String,
    pub size_estimate: u64,
    /// 安装位置（注册表 InstallLocation，可能为空）
    pub install_location: String,
}

fn enum_root(root: HKEY, path: &str, label: &str, out: &mut Vec<InstalledApp>) {
    let hive = match RegKey::predef(root).open_subkey(path) {
        Ok(k) => k,
        Err(_) => return,
    };
    for sub in hive.enum_keys().flatten() {
        let Ok(k) = hive.open_subkey(&sub) else { continue };
        let name: String = k.get_value("DisplayName").unwrap_or_default();
        if name.trim().is_empty() {
            continue;
        }
        let version: String = k.get_value("DisplayVersion").unwrap_or_default();
        let publisher: String = k.get_value("Publisher").unwrap_or_default();
        let uninstaller: String = k
            .get_value::<String, _>("QuietUninstallString")
            .or_else(|_| k.get_value::<String, _>("UninstallString"))
            .unwrap_or_default();
        let size_estimate = k.get_value::<u32, _>("EstimatedSize").map(|v| v as u64 * 1024).unwrap_or(0);
        let install_location: String = k.get_value("InstallLocation").unwrap_or_default();
        out.push(InstalledApp {
            name,
            version,
            publisher,
            uninstaller,
            key: format!("{}\\{}\\{}", label, path, sub),
            size_estimate,
            install_location,
        });
    }
}

/// 枚举已安装软件（HKLM + WOW6432Node + HKCU 的 Uninstall 键）
#[tauri::command]
pub(crate) fn list_installed_inner() -> Vec<InstalledApp> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (root, path, label) in [
        (HKEY_LOCAL_MACHINE, UNINSTALL_PATH, "HKLM"),
        (HKEY_LOCAL_MACHINE, WOW_UNINSTALL_PATH, "HKLM"),
        (HKEY_CURRENT_USER, UNINSTALL_PATH, "HKCU"),
    ] {
        let mut tmp = Vec::new();
        enum_root(root, path, label, &mut tmp);
        for item in tmp {
            if seen.insert(item.key.clone()) {
                out.push(item);
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Tauri 命令入口（线程池执行，避免阻塞主线程）
#[tauri::command]
pub async fn list_installed() -> Vec<InstalledApp> {
    list_installed_inner()
}


/// 残留项
#[derive(Debug, Serialize, Deserialize)]
pub struct ResidualItem {
    /// "file" | "dir" | "reg"
    pub kind: String,
    pub target: String,
}

fn walk_fs(dir: &Path, depth: u32, kw: &str, out: &mut Vec<ResidualItem>) {
    if depth > 2 || out.len() >= 100 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let fname = e.file_name().to_string_lossy().to_lowercase();
        if fname.contains(kw) {
            out.push(ResidualItem {
                kind: if ft.is_dir() { "dir" } else { "file" }.into(),
                target: e.path().to_string_lossy().into_owned(),
            });
            if out.len() >= 100 {
                return;
            }
        } else if ft.is_dir() {
            walk_fs(&e.path(), depth + 1, kw, out);
        }
    }
}

/// 按名称关键字（前 2 个词）扫描残留：Program Files / ProgramData / LOCALAPPDATA / Uninstall 注册表键
#[tauri::command]
pub async fn scan_residual(name: String) -> Vec<ResidualItem> {
    let out = scan_residual_inner(name.clone());
    let dirs = crate::dirs::load_dirs();
    crate::logs::op_log(&dirs, "uninstall", "scan_residual", &format!("扫描 \"{}\" 残留: 命中 {} 项", name, out.len()));
    out
}

fn scan_residual_inner(name: String) -> Vec<ResidualItem> {
    let words: Vec<&str> = name.split_whitespace().filter(|w| w.chars().count() >= 2).collect();
    let kw = if words.is_empty() {
        name.trim().to_lowercase()
    } else {
        words[..words.len().min(2)].join(" ").to_lowercase()
    };
    if kw.is_empty() {
        return vec![];
    }

    let mut out = Vec::new();
    let mut roots: Vec<String> = vec![
        r"C:\Program Files".into(),
        r"C:\Program Files (x86)".into(),
    ];
    if let Ok(pd) = std::env::var("ProgramData") {
        roots.push(pd);
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        roots.push(la);
    }
    for root in roots {
        walk_fs(Path::new(&root), 0, &kw, &mut out);
        if out.len() >= 100 {
            break;
        }
    }
    out.truncate(100);

    // 注册表 Uninstall 键扫描
    for (root, path, label) in [
        (HKEY_LOCAL_MACHINE, UNINSTALL_PATH, "HKLM"),
        (HKEY_LOCAL_MACHINE, WOW_UNINSTALL_PATH, "HKLM"),
        (HKEY_CURRENT_USER, UNINSTALL_PATH, "HKCU"),
    ] {
        let Ok(hive) = RegKey::predef(root).open_subkey(path) else { continue };
        for sub in hive.enum_keys().flatten() {
            if let Ok(k) = hive.open_subkey(&sub) {
                let dn: String = k.get_value("DisplayName").unwrap_or_default();
                if dn.to_lowercase().contains(&kw) {
                    out.push(ResidualItem {
                        kind: "reg".into(),
                        target: format!("{}\\{}\\{}", label, path, sub),
                    });
                    if out.len() >= 200 {
                        return out;
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 进程检测与关闭
// ---------------------------------------------------------------------------

use std::os::windows::process::CommandExt;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 按名称模糊查找运行中的进程（忽略大小写，带不带 .exe 均可匹配），最多返回 20 个 pid
#[tauri::command]
pub async fn process_running(name: String) -> Vec<u32> {
    let mut kw = name.trim().to_lowercase();
    if kw.is_empty() {
        return vec![];
    }
    if let Some(stripped) = kw.strip_suffix(".exe") {
        kw = stripped.to_string();
    }

    use sysinfo::ProcessesToUpdate;
    use sysinfo::System as SysSystem;
    let mut sys = SysSystem::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let mut out: Vec<u32> = sys
        .processes()
        .values()
        .filter(|p| {
            let pname = p.name().to_string_lossy().to_lowercase();
            let pname = pname.strip_suffix(".exe").unwrap_or(&pname);
            pname.contains(&kw)
        })
        .map(|p| p.pid().as_u32())
        .collect();
    out.sort();
    out.truncate(20);
    out
}

/// 结束进程结果
#[derive(Serialize)]
pub struct KillResult {
    pub killed: Vec<u32>,
    pub failed: Vec<u32>,
}

/// 按 pid 逐个强制结束进程：先 taskkill /F，失败再用 sysinfo kill 兜底
#[tauri::command]
pub async fn kill_process(pids: Vec<u32>) -> KillResult {
    let dirs = crate::dirs::load_dirs();
    let mut killed = Vec::new();
    let mut failed = Vec::new();

    use sysinfo::ProcessesToUpdate;
    use sysinfo::System as SysSystem;
    let mut sys = SysSystem::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    for pid in pids {
        // taskkill 更稳（含权限提升处理），失败再用 sysinfo 兜底
        let ok = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let ok = ok || {
            sys.process(sysinfo::Pid::from_u32(pid))
                .map(|p| p.kill())
                .unwrap_or(false)
        };
        if ok {
            killed.push(pid);
        } else {
            failed.push(pid);
        }
    }

    crate::logs::op_log(
        &dirs,
        "uninstall",
        "kill_process",
        &format!("结束进程: 成功 {:?} 失败 {:?}", killed, failed),
    );
    KillResult { killed, failed }
}

fn delete_reg_key(path: &str) -> Result<(), String> {
    let Some((root_str, rest)) = path.split_once('\\') else {
        return Err(format!("注册表路径无效: {}", path));
    };
    let root = match root_str.to_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        _ => return Err(format!("不支持的注册表根键: {}", root_str)),
    };
    let rest = rest.trim_start_matches('\\');
    let Some((parent, key_name)) = rest.rsplit_once('\\') else {
        return Err(format!("注册表路径无效: {}", path));
    };
    let pk = RegKey::predef(root)
        .open_subkey_with_flags(parent, KEY_ALL_ACCESS)
        .map_err(|e| format!("{}: {}", path, e))?;
    pk.delete_subkey_all(key_name).map_err(|e| format!("{}: {}", path, e))
}

/// 删除残留（文件 / 目录 / 注册表键）
#[tauri::command]
pub fn remove_residual(items: Vec<ResidualItem>) -> Result<(), String> {
    let dirs = crate::dirs::load_dirs();
    let count = items.len();
    let mut errs = Vec::new();
    for item in items {
        let r = match item.kind.as_str() {
            "file" => fs::remove_file(&item.target).map_err(|e| format!("{}: {}", item.target, e)),
            "dir" => fs::remove_dir_all(&item.target).map_err(|e| format!("{}: {}", item.target, e)),
            "reg" => delete_reg_key(&item.target),
            _ => Err(format!("未知残留类型: {}", item.kind)),
        };
        if let Err(e) = r {
            errs.push(e);
        }
    }
    let r = if errs.is_empty() { Ok(()) } else { Err(errs.join("; ")) };
    let outcome = match &r {
        Ok(_) => "成功".to_string(),
        Err(e) => format!("部分失败: {}", e),
    };
    crate::logs::op_log(&dirs, "uninstall", "remove_residual", &format!("清理 {} 项残留: {}", count, outcome));
    r
}
