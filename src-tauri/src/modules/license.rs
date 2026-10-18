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
    /// 完整版本号，如 10.0.26100
    pub build: String,
    /// 激活渠道（Retail / OEM / Volume 等，取自 SLsvc 注册表）
    pub channel: String,
    /// 系统安装时间（yyyy-MM-dd，取自注册表 InstallDate）
    pub install_date: String,
    /// 剩余宽限期天数（仅宽限状态有意义，0 表示无）
    pub grace_minutes: u64,
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

/// InstallDate 是 Unix 秒（可能为 0）。
/// 注册表存的是 UTC 秒，必须按本地时区换算，否则东八区会差一天
/// （1740349499 是 UTC 2025-02-23 20:24，本地应是 2025-02-24）。
fn format_install_date(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => String::new(),
    }
}

/// 从注册表读取系统版本信息
fn read_os_info() -> (String, String, String, String, String, String) {
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
            let build = k
                .get_value::<String, _>("CurrentBuildNumber")
                .unwrap_or_else(|_| empty());
            let channel = k.get_value::<String, _>("EditionID").unwrap_or_else(|_| empty());
            // InstallDate 在不同系统上可能是 DWORD 也可能是字符串，两种都试
            let install_ts = k
                .get_value::<u32, _>("InstallDate")
                .map(|v| v as i64)
                .or_else(|_| {
                    k.get_value::<String, _>("InstallDate")
                        .map(|s| s.trim().parse::<i64>().unwrap_or(0))
                })
                .unwrap_or(0);
            (edition, version, product_id, build, channel, format_install_date(install_ts))
        }
        Err(_) => (empty(), empty(), empty(), empty(), empty(), empty()),
    }
}

/// 查询 Windows 许可证状态（只读，无任何写操作）
#[tauri::command]
pub async fn license_status() -> LicenseStatus {
    // 内部要起 PowerShell，必须离开异步线程
    tauri::async_runtime::spawn_blocking(license_status_sync)
        .await
        .unwrap_or_else(|_| LicenseStatus {
            os_edition: String::new(),
            os_version: String::new(),
            status_text: "查询失败".into(),
            partial_key: String::new(),
            product_id: String::new(),
            licensed: false,
            build: String::new(),
            channel: String::new(),
            install_date: String::new(),
            grace_minutes: 0,
        })
}

/// 同步版本：供导出报告等内部调用。
pub fn license_status_sync() -> LicenseStatus {
    let (os_edition, os_version, reg_product_id, build, channel, install_date) = read_os_info();

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
    // 宽限期天数目前未从 CIM 读取，保留字段给前端展示预留
    let grace_minutes: u64 = 0;

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
        build,
        channel,
        install_date,
        grace_minutes,
    }
}

#[cfg(test)]
mod tests {
    use super::{map_license_status, LicenseStatus};

    #[test]
    fn license_status_codes() {
        assert_eq!(map_license_status(0), "未授权");
        assert_eq!(map_license_status(1), "已授权");
        assert_eq!(map_license_status(5), "未激活警告");
        assert_eq!(map_license_status(99), "未知状态");
    }

    #[test]
    fn license_json_is_camel_case() {
        // 前端 LicenseView 读 osEdition / statusText / partialKey / productId
        let v = serde_json::to_value(LicenseStatus {
            os_edition: "Windows 11 Pro".into(),
            os_version: "24H2".into(),
            status_text: "已授权".into(),
            partial_key: "ABCDE".into(),
            product_id: "00330".into(),
            licensed: true,
            build: "26100".into(),
            channel: "Professional".into(),
            install_date: "2025-01-02".into(),
            grace_minutes: 0,
        })
        .unwrap();
        assert_eq!(v["osEdition"], "Windows 11 Pro");
        assert_eq!(v["osVersion"], "24H2");
        assert_eq!(v["statusText"], "已授权");
        assert_eq!(v["partialKey"], "ABCDE");
        assert_eq!(v["productId"], "00330");
        assert_eq!(v["licensed"], true);
        assert_eq!(v["build"], "26100");
        assert_eq!(v["installDate"], "2025-01-02");
        assert!(v.get("os_edition").is_none());
    }

    #[test]
    fn install_date_formats_and_guards_zero() {
        use super::format_install_date;
        assert_eq!(format_install_date(0), "");
        assert_eq!(format_install_date(-1), "");
        // 2024-03-06 00:00:00 UTC：东八区是当天 08:00，日期不变；西半球可能退到前一天。
        // 所以这里按本地时区算期望值，不硬编码，避免测试随机器时区失败。
        use chrono::TimeZone;
        let expected = chrono::Local
            .timestamp_opt(1_709_683_200, 0)
            .single()
            .unwrap()
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(format_install_date(1_709_683_200), expected);
    }

    #[test]
    fn install_date_uses_local_timezone() {
        // 本机实测值：注册表存 1740349499（UTC 2025-02-23 20:24），
        // 东八区应显示 2025-02-24。之前直接用 UTC 天数换算，少了一天。
        use super::format_install_date;
        use chrono::TimeZone;
        let ts = 1_740_349_499i64;
        let expected = chrono::Local
            .timestamp_opt(ts, 0)
            .single()
            .unwrap()
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(format_install_date(ts), expected);
        // 换算结果必须落在 2025-02-23 或 24 之间，不能跑到别的日期
        assert!(
            expected == "2025-02-23" || expected == "2025-02-24",
            "unexpected local date: {expected}"
        );
    }
}
