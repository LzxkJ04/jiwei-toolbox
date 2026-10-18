//! 公共模块：环境检测、设置存储、自启动管理、共享路径工具

use serde::Serialize;
use std::path::PathBuf;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE};
use winreg::RegKey;

pub const APP_DIR_NAME: &str = "极维工具箱";

/// Documents 目录（手拼 known folder：%USERPROFILE%\Documents）
pub fn user_documents() -> PathBuf {
    std::env::var("USERPROFILE")
        .map(|p| PathBuf::from(p).join("Documents"))
        .unwrap_or_else(|_| PathBuf::from(r"C:\Users\Public\Documents"))
}

/// %LOCALAPPDATA%
pub fn local_appdata() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Users\Default\AppData\Local"))
}

/// 去除 UTF-8 BOM（utf-8-sig 兼容）
pub fn strip_bom(s: String) -> String {
    s.trim_start_matches('\u{feff}').to_string()
}

/// 管理员检测：OpenProcessToken + TokenElevation
pub fn is_admin() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret_len: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elev as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);
        ok != 0 && elev.TokenIsElevated != 0
    }
}

/// WebView2 运行时检测：EdgeUpdate Clients 注册表 pv 值非空
pub fn has_webview2() -> bool {
    const CLIENT_KEY: &str = r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    let candidates = [
        (HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\WOW6432Node\{}", CLIENT_KEY)),
        (HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\{}", CLIENT_KEY)),
        (HKEY_CURRENT_USER, format!(r"Software\{}", CLIENT_KEY)),
    ];
    for (root, path) in candidates {
        if let Ok(k) = RegKey::predef(root).open_subkey_with_flags(&path, KEY_QUERY_VALUE) {
            if let Ok(pv) = k.get_value::<String, _>("pv") {
                if !pv.trim().is_empty() {
                    return true;
                }
            }
        }
    }
    false
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvStatus {
    pub is_admin: bool,
    pub has_webview2: bool,
}

/// 环境状态：管理员 / WebView2
#[tauri::command]
pub fn env_status() -> EnvStatus {
    EnvStatus { is_admin: is_admin(), has_webview2: has_webview2() }
}

// ---------- 自启动（HKCU Run） ----------

const RUN_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_NAME: &str = "极维工具箱";

/// 本程序是否已设置开机自启
#[tauri::command]
pub fn get_autostart() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_PATH)
        .and_then(|k| k.get_value::<String, _>(AUTOSTART_NAME))
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

/// 设置/取消开机自启（写入 HKCU Run）
#[tauri::command]
pub fn set_autostart(enable: bool) -> Result<(), String> {
    let run = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_PATH, KEY_SET_VALUE)
        .map_err(|e| e.to_string())?;
    if enable {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        run.set_value(AUTOSTART_NAME, &exe.to_string_lossy().to_string())
            .map_err(|e| e.to_string())
    } else {
        run.delete_value(AUTOSTART_NAME).or(Ok(()))
    }
}

// ---------- 设置存储（config\settings.json，UTF-8 无 BOM） ----------

fn settings_file() -> PathBuf {
    PathBuf::from(crate::dirs::load_dirs().config).join("settings.json")
}

/// 读取设置项（不存在返回空字符串）
#[tauri::command]
pub fn get_setting(key: String) -> String {
    let path = settings_file();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return String::new(),
    };
    let text = strip_bom(String::from_utf8_lossy(&bytes).into_owned());
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    match v.get(&key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// 写入设置项
#[tauri::command]
pub fn set_setting(key: String, value: String) -> Result<(), String> {
    write_setting_raw(&key, serde_json::Value::String(value))
}

/// 读取设置项原始 JSON 值（后端内部使用，支持布尔/对象等非字符串类型）
pub fn read_setting_raw(key: &str) -> Option<serde_json::Value> {
    let path = settings_file();
    let bytes = std::fs::read(&path).ok()?;
    let text = strip_bom(String::from_utf8_lossy(&bytes).into_owned());
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get(key).cloned()
}

/// 写入设置项原始 JSON 值（后端内部使用）
pub fn write_setting_raw(key: &str, value: serde_json::Value) -> Result<(), String> {
    let path = settings_file();
    let mut obj: serde_json::Map<String, serde_json::Value> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_str::<serde_json::Value>(&strip_bom(String::from_utf8_lossy(&b).into_owned())).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    obj.insert(key.to_string(), value);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let out = serde_json::to_string_pretty(&serde_json::Value::Object(obj)).map_err(|e| e.to_string())?;
    std::fs::write(&path, out).map_err(|e| e.to_string())
}

/// "关闭到托盘"开关（后端内部使用，默认 true）
pub fn close_to_tray_enabled() -> bool {
    read_setting_raw("close_to_tray")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 读取"关闭到托盘"开关
#[tauri::command]
pub fn get_close_to_tray() -> bool {
    close_to_tray_enabled()
}

/// 设置"关闭到托盘"开关
#[tauri::command]
pub fn set_close_to_tray(enable: bool) -> Result<(), String> {
    write_setting_raw("close_to_tray", serde_json::Value::Bool(enable))
}

// ---------- 收藏（settings.json favorites 数组） ----------

fn read_favorites() -> Vec<String> {
    read_setting_raw("favorites")
        .and_then(|v| v.as_array().cloned())
        .map(|a| a.into_iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

/// 获取收藏的软件 id 列表
#[tauri::command]
pub fn get_favorites() -> Vec<String> {
    read_favorites()
}

/// 切换收藏状态（已收藏则取消，未收藏则加入），返回更新后的列表
#[tauri::command]
pub fn toggle_favorite(id: String) -> Result<Vec<String>, String> {
    let mut list = read_favorites();
    if let Some(pos) = list.iter().position(|x| x == &id) {
        list.remove(pos);
    } else {
        list.push(id);
    }
    write_setting_raw(
        "favorites",
        serde_json::Value::Array(list.iter().cloned().map(serde_json::Value::String).collect()),
    )?;
    Ok(list)
}
