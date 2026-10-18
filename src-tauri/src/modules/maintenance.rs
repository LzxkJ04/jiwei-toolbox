//! 系统维护：回收站、电源计划、启动耗时

use serde::Serialize;
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn decode(bytes: &[u8]) -> String {
    let bytes = if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        &bytes[2..]
    } else {
        bytes
    };
    let utf16 = bytes.len() >= 4 && bytes.len() % 2 == 0 && bytes[1] == 0 && bytes[3] == 0;
    let text = if utf16 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    text.trim_start_matches('\u{feff}').trim().to_string()
}

/// 清空回收站。返回是否成功。
#[tauri::command]
pub async fn empty_recycle_bin() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let out = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Clear-RecycleBin -Force -ErrorAction Stop",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("执行失败：{e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let msg = decode(&out.stderr);
            Err(if msg.is_empty() {
                "回收站可能已经是空的，或需要管理员权限".to_string()
            } else {
                msg
            })
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct PowerPlan {
    pub id: String,
    pub name: String,
    pub active: bool,
}

/// 列出电源计划，并标出当前使用的那个。
#[tauri::command]
pub async fn list_power_plans() -> Vec<PowerPlan> {
    tauri::async_runtime::spawn_blocking(|| {
        let out = std::process::Command::new("powercfg")
            .arg("/list")
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        let Ok(out) = out else { return vec![] };
        let text = decode(&out.stdout);
        let mut plans = Vec::new();
        for line in text.lines() {
            // 形如：Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (平衡) *
            let Some(rest) = line.split_once("GUID:") else { continue };
            let rest = rest.1.trim();
            let mut parts = rest.splitn(2, char::is_whitespace);
            let Some(id) = parts.next() else { continue };
            let tail = parts.next().unwrap_or("").trim();
            let active = tail.ends_with('*');
            let name = tail
                .trim_end_matches('*')
                .trim()
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim()
                .to_string();
            if id.is_empty() {
                continue;
            }
            plans.push(PowerPlan {
                id: id.to_string(),
                name: if name.is_empty() { id.to_string() } else { name },
                active,
            });
        }
        plans
    })
    .await
    .unwrap_or_default()
}

/// 切换电源计划（按 GUID）。
#[tauri::command]
pub async fn set_power_plan(id: String) -> Result<(), String> {
    let guid = id.trim().to_string();
    // GUID 只允许十六进制与连字符，避免拼进命令行
    if guid.is_empty()
        || !guid
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-' || c == '{' || c == '}')
    {
        return Err("电源计划 ID 非法".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let out = std::process::Command::new("powercfg")
            .args(["/setactive", &guid])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("执行失败：{e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let msg = decode(&out.stderr);
            Err(if msg.is_empty() { "切换失败，可能需要管理员权限".to_string() } else { msg })
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct UptimeInfo {
    /// 系统启动时间（Unix 秒）
    pub boot_ts: u64,
    /// 当前已运行秒数
    pub uptime: u64,
    /// 开机自检到桌面可用的近似秒数（读事件日志，取不到为 0）
    pub boot_seconds: u64,
}

/// 运行时长与上次开机耗时。
#[tauri::command]
pub async fn uptime_info() -> UptimeInfo {
    tauri::async_runtime::spawn_blocking(|| {
        let uptime = sysinfo::System::uptime();
        let boot_ts = sysinfo::System::boot_time();
        let boot_seconds = read_boot_seconds();
        UptimeInfo { boot_ts, uptime, boot_seconds }
    })
    .await
    .unwrap_or(UptimeInfo { boot_ts: 0, uptime: 0, boot_seconds: 0 })
}

/// 毫秒 → 秒（四舍五入，最少 1 秒；0 保持 0）
fn ms_to_secs(ms: u64) -> u64 {
    if ms == 0 {
        0
    } else {
        ((ms + 500) / 1000).max(1)
    }
}

/// 从诊断性能事件日志读最近一次开机的启动耗时（秒）。读不到返回 0。
/// 事件里的 BootTime 单位是毫秒（本机实测 16375），必须换算成秒。
fn read_boot_seconds() -> u64 {
    let script = r#"
$t = 0
try {
  $e = Get-WinEvent -LogName 'Microsoft-Windows-Diagnostics-Performance/Operational' -MaxEvents 40 -ErrorAction Stop |
       Where-Object { $_.Id -eq 100 } | Select-Object -First 1
  if ($e) {
    $x = [xml]$e.ToXml()
    foreach ($d in $x.Event.EventData.Data) {
      if ($d.Name -eq 'BootTime') { $t = [int]$d.'#text' }
    }
  }
} catch {}
[int]$t
"#;
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(out) = out else { return 0 };
    let ms = decode(&out.stdout)
        .trim()
        .trim_matches(|c: char| !c.is_ascii_digit())
        .parse::<u64>()
        .unwrap_or(0);
    if ms == 0 {
        0
    } else {
        // 毫秒 → 秒（四舍五入），最少 1 秒
        ms_to_secs(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, ms_to_secs};

    #[test]
    fn decode_handles_utf8_and_utf16() {
        assert_eq!(decode(b"hello"), "hello");
        assert_eq!(decode(&[0xFF, 0xFE, b'h', 0, b'i', 0]), "hi");
        // BOM 要去掉
        assert_eq!(decode("\u{feff}x".as_bytes()), "x");
    }

    #[test]
    fn boot_time_millis_become_seconds() {
        // 实测事件里的 BootTime 是毫秒，16375 应该显示成约 16 秒而不是 16375 秒
        assert_eq!(ms_to_secs(16375), 16);
        assert_eq!(ms_to_secs(400), 1);
        assert_eq!(ms_to_secs(0), 0);
    }
}
