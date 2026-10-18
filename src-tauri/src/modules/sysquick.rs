//! 快捷工具：端口占用、系统工具入口、定时关机

use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::sync::Mutex;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn ps_run(script: &str) -> String {
    let full = format!("[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; {}", script);
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &full])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) => {
            let bytes = o.stdout;
            let bytes = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(&bytes);
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
        Err(_) => String::new(),
    }
}

fn ps_json(script: &str) -> Option<serde_json::Value> {
    let text = ps_run(script);
    if let Some(i) = text.find(['{', '[']) {
        if let Ok(v) = serde_json::from_str(&text[i..]) {
            return Some(v);
        }
    }
    serde_json::from_str(&text).ok()
}

fn as_array(v: Option<serde_json::Value>) -> Vec<serde_json::Value> {
    match v {
        Some(serde_json::Value::Array(a)) => a,
        Some(serde_json::Value::Null) | None => vec![],
        Some(x) => vec![x],
    }
}

#[derive(Serialize, Clone)]
pub struct PortRow {
    /// "TCP" / "UDP"
    pub proto: String,
    pub addr: String,
    pub port: u32,
    pub pid: u32,
    pub name: String,
}

/// 本机正在监听的 TCP / UDP 端口及占用进程。
#[tauri::command]
pub async fn list_ports() -> Vec<PortRow> {
    tauri::async_runtime::spawn_blocking(|| {
        let script = r#"
$names = @{}
try { Get-Process | ForEach-Object { $names[[int]$_.Id] = $_.ProcessName } } catch {}
$rows = @()
try {
  Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | ForEach-Object {
    $p = [int]$_.OwningProcess
    $rows += @{ proto = 'TCP'; addr = [string]$_.LocalAddress; port = [int]$_.LocalPort; pid = $p; name = [string]$names[$p] }
  }
} catch {}
try {
  Get-NetUDPEndpoint -ErrorAction SilentlyContinue | ForEach-Object {
    $p = [int]$_.OwningProcess
    $rows += @{ proto = 'UDP'; addr = [string]$_.LocalAddress; port = [int]$_.LocalPort; pid = $p; name = [string]$names[$p] }
  }
} catch {}
ConvertTo-Json -InputObject @($rows) -Compress -Depth 3
"#;
        let mut out: Vec<PortRow> = as_array(ps_json(script))
            .into_iter()
            .map(|r| PortRow {
                proto: r.get("proto").and_then(|x| x.as_str()).unwrap_or("TCP").to_string(),
                addr: r.get("addr").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                port: r.get("port").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                pid: r.get("pid").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                name: r.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            })
            .filter(|p| p.port > 0)
            .collect();
        // 同一端口可能被 IPv4/IPv6 各监听一次，按协议+端口+进程去重
        out.sort_by(|a, b| a.port.cmp(&b.port).then(a.proto.cmp(&b.proto)));
        out.dedup_by(|a, b| a.port == b.port && a.proto == b.proto && a.pid == b.pid);
        out
    })
    .await
    .unwrap_or_default()
}

/// 系统工具入口白名单。只接受下表里的 id，不接受任意命令。
fn tool_target(id: &str) -> Option<(&'static str, &'static str)> {
    let pair = match id {
        "control" => ("控制面板", "control.exe"),
        "appwiz" => ("程序和功能", "appwiz.cpl"),
        "devmgmt" => ("设备管理器", "devmgmt.msc"),
        "services" => ("服务", "services.msc"),
        "taskmgr" => ("任务管理器", "taskmgr.exe"),
        "regedit" => ("注册表编辑器", "regedit.exe"),
        "sysprops" => ("系统属性", "SystemPropertiesAdvanced.exe"),
        "network" => ("网络连接", "ncpa.cpl"),
        "powercfg" => ("电源选项", "powercfg.cpl"),
        "diskmgmt" => ("磁盘管理", "diskmgmt.msc"),
        "eventvwr" => ("事件查看器", "eventvwr.msc"),
        "resmon" => ("资源监视器", "resmon.exe"),
        "msinfo" => ("系统信息", "msinfo32.exe"),
        "firewall" => ("防火墙", "firewall.cpl"),
        "timedate" => ("日期时间", "timedate.cpl"),
        "display" => ("显示设置", "desk.cpl"),
        "sound" => ("声音", "mmsys.cpl"),
        "userpass" => ("用户账户", "netplwiz.exe"),
        "env" => ("环境变量面板", "SystemPropertiesAdvanced.exe"),
        "temp" => ("临时文件夹", "%TEMP%"),
        "startupfolder" => ("启动文件夹", "shell:startup"),
        "hostsfolder" => ("Hosts 所在目录", r"C:\Windows\System32\drivers\etc"),
        "fonts" => ("字体设置", "shell:fonts"),
        _ => return None,
    };
    Some(pair)
}

/// 打开 Windows 自带的系统工具 / 面板 / 文件夹。只接受白名单 id。
#[tauri::command]
pub async fn open_system_tool(id: String) -> Result<String, String> {
    let (label, target) = tool_target(id.trim()).ok_or_else(|| "不支持的工具".to_string())?;
    let target = target.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &target])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("打开失败：{e}"))?;
        Ok(label.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

static SHUTDOWN_PENDING: Mutex<Option<(u64, String)>> = Mutex::new(None);

/// 定时关机 / 重启 / 注销。seconds 为 0 表示立即执行。
#[tauri::command]
pub async fn schedule_shutdown(seconds: u64, mode: String) -> Result<(), String> {
    let flag = match mode.as_str() {
        "shutdown" => "/s",
        "restart" => "/r",
        "logoff" => "/l",
        _ => return Err("未知的关机模式".into()),
    };
    let secs = seconds.min(24 * 3600);
    tauri::async_runtime::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("shutdown");
        cmd.creation_flags(CREATE_NO_WINDOW);
        if flag == "/l" {
            // 注销不接受 /t
            cmd.arg("/l");
        } else {
            cmd.args([flag, "/t", &secs.to_string()]);
            if secs > 0 {
                cmd.args(["/c", "极维工具箱 定时任务"]);
            }
        }
        let out = cmd.output().map_err(|e| format!("执行失败：{e}"))?;
        if out.status.success() {
            *SHUTDOWN_PENDING.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((secs, flag.trim_start_matches('/').to_string()));
            Ok(())
        } else {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if msg.is_empty() {
                "执行失败，可能需要管理员权限".to_string()
            } else {
                msg
            })
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 取消已排定的关机。
#[tauri::command]
pub async fn cancel_shutdown() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let out = std::process::Command::new("shutdown")
            .arg("/a")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("执行失败：{e}"))?;
        if out.status.success() {
            *SHUTDOWN_PENDING.lock().unwrap_or_else(|e| e.into_inner()) = None;
            Ok(())
        } else {
            Err("没有正在等待的关机任务".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::tool_target;

    #[test]
    fn unknown_tool_id_is_rejected() {
        assert!(tool_target("control").is_some());
        assert!(tool_target("devmgmt").is_some());
        assert!(tool_target("").is_none());
        assert!(tool_target("cmd /c del").is_none());
        assert!(tool_target("../../windows").is_none());
    }

    #[test]
    fn whitelist_targets_have_no_shell_metachars() {
        for id in [
            "control", "appwiz", "devmgmt", "services", "taskmgr", "regedit", "sysprops",
            "network", "powercfg", "diskmgmt", "eventvwr", "resmon", "msinfo", "firewall",
            "timedate", "display", "sound", "userpass", "env", "temp", "startupfolder",
            "hostsfolder", "fonts",
        ] {
            let (_, target) = tool_target(id).expect("白名单项应存在");
            assert!(
                !target.contains(';') && !target.contains('&') && !target.contains('|'),
                "{id} 的目标含 shell 元字符"
            );
        }
    }
}
