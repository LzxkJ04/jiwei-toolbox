//! Windows 许可证状态查询（只读，不含任何激活/破解逻辑）
//!
//! 数据来源：
//! - PowerShell SoftwareLicensingProduct（Windows 应用ID 过滤 + PartialProductKey 非空）
//! - 注册表 HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion（系统版本信息）

use serde::Serialize;
use std::os::windows::process::CommandExt;
use winreg::enums::HKEY_LOCAL_MACHINE;
use winreg::RegKey;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Windows 授权的 ApplicationID
const WINDOWS_APP_ID: &str = "55c92734-d682-4d71-983e-d6ec3f16059f";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseStatus {
    pub os_edition: String,
    pub os_version: String,
    pub status_text: String,
    pub partial_key: String,
    pub product_id: String,
    pub licensed: bool,
}

/// LicenseStatus 数值 → 中文描述（SLAID 标准 0-6）
fn map_license_status(code: i64) -> &'static str {
    match code {
        0 => "未授权",
        1 => "已授权",
        2 => "初始宽限期",
        3 => "宽限期",
        4 => "非正版宽限期",
        5 => "未激活警告",
        6 => "扩展宽限期",
        _ => "未知状态",
    }
}

/// 从注册表读取系统版本信息
fn read_os_info() -> (String, String, String) {
    let empty = || String::new();
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    match key {
        Ok(k) => {
            let edition = k.get_value::<String, _>("ProductName").unwrap_or_else(|_| empty());
            let version = k
                .get_value::<String, _>("DisplayVersion")
                .or_else(|_| k.get_value::<String, _>("ReleaseId"))
                .unwrap_or_else(|_| empty());
            let product_id = k.get_value::<String, _>("ProductId").unwrap_or_else(|_| empty());
            (edition, version, product_id)
        }
        Err(_) => (empty(), empty(), empty()),
    }
}

/// 查询 Windows 许可证状态（只读，无任何写操作）
#[tauri::command]
pub fn license_status() -> LicenseStatus {
    let (os_edition, os_version, reg_product_id) = read_os_info();

    // PowerShell CIM 查询：PartialProductKey 非空且属于 Windows 应用
    let ps_script = format!(
        "Get-CimInstance SoftwareLicensingProduct -Filter \"PartialProductKey IS NOT NULL AND ApplicationID='{}'\" \
         | Select-Object Name,Description,LicenseStatus,PartialProductKey,ProductKeyID \
         | ConvertTo-Json -Compress",
        WINDOWS_APP_ID
    );
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let mut status_code: i64 = -1;
    let mut partial_key = String::new();
    let mut product_id = reg_product_id.clone();

    if let Ok(out) = output {
        if out.status.success() {
            let text = crate::common::strip_bom(String::from_utf8_lossy(&out.stdout).into_owned());
            let text = text.trim().to_string();
            if !text.is_empty() {
                // ConvertTo-Json：单条为对象，多条为数组
                let parsed: serde_json::Value = serde_json::from_str(&text)
                    .unwrap_or(serde_json::Value::Null);
                let items: Vec<serde_json::Value> = match parsed {
                    serde_json::Value::Array(a) => a,
                    obj @ serde_json::Value::Object(_) => vec![obj],
                    _ => Vec::new(),
                };
                // 优先取 Name/Description 含 "Windows" 的条目
                let entry = items
                    .iter()
                    .find(|it| {
                        it.get("Name").and_then(|v| v.as_str()).map(|s| s.contains("Windows")).unwrap_or(false)
                    })
                    .or_else(|| items.first());
                if let Some(e) = entry {
                    status_code = e.get("LicenseStatus").and_then(|v| v.as_i64()).unwrap_or(-1);
                    if let Some(pk) = e.get("PartialProductKey").and_then(|v| v.as_str()) {
                        partial_key = pk.to_string();
                    }
                    if let Some(pid) = e.get("ProductKeyID").and_then(|v| v.as_str()) {
                        let pid = pid.trim();
                        if !pid.is_empty() {
                            product_id = pid.to_string();
                        }
                    }
                }
            }
        }
    }

    // CIM 查询失败时降级为"未知"
    let status_text = if status_code < 0 { "查询失败" } else { map_license_status(status_code) };

    LicenseStatus {
        os_edition,
        os_version,
        status_text: status_text.to_string(),
        partial_key,
        product_id,
        licensed: status_code == 1,
    }
}
