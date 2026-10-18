//! 系统监控：实时占用、网络速率、进程管理、网络诊断

use serde::Serialize;
use serde_json::json;
use std::sync::Mutex;
use std::time::Instant;

// ---------- 实时占用 + 网络速率 ----------

struct UsageState {
    sys: sysinfo::System,
    net: sysinfo::Networks,
    last: Instant,
    down_bps: f64,
    up_bps: f64,
    /// 首次调用要补一次基线采样，否则 CPU 占用会假性 100%
    warm: bool,
    /// 进程数不必每轮都全量刷新
    proc_ticks: u32,
    proc_count: usize,
}

static USAGE: Mutex<Option<UsageState>> = Mutex::new(None);

fn with_usage<T>(f: impl FnOnce(&mut UsageState) -> T) -> Option<T> {
    let mut guard = USAGE.lock().unwrap_or_else(|e| e.into_inner());
    let st = guard.get_or_insert_with(|| UsageState {
        sys: sysinfo::System::new(),
        net: sysinfo::Networks::new_with_refreshed_list(),
        last: Instant::now(),
        down_bps: 0.0,
        up_bps: 0.0,
        warm: false,
        proc_ticks: 0,
        proc_count: 0,
    });
    Some(f(st))
}

fn collect_usage() -> serde_json::Value {
    let fallback = json!({
        "cpu_pct": 0.0,
        "per_core": [],
        "mem_used": 0u64,
        "mem_total": 0u64,
        "swap_used": 0u64,
        "swap_total": 0u64,
        "down_bps": 0.0,
        "up_bps": 0.0,
        "processes": 0usize,
        "uptime": 0u64,
    });
    with_usage(|st| {
        st.sys.refresh_memory();
        if st.sys.cpus().is_empty() {
            st.sys.refresh_cpu_all();
        }
        if !st.warm {
            // 两次采样之间的差值才有意义，先等一个最小间隔
            std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
            st.warm = true;
        }
        st.sys.refresh_cpu_usage();

        // 进程枚举只取数量，每 4 轮（约 6 秒）刷新一次，避免每轮都扫全表
        if st.proc_ticks % 4 == 0 {
            use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
            st.proc_count = st.sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing(),
            );
        }
        st.proc_ticks = st.proc_ticks.wrapping_add(1);

        let now = Instant::now();
        let elapsed = now.duration_since(st.last).as_secs_f64();
        st.net.refresh(true);
        if elapsed >= 0.4 {
            let mut down = 0u64;
            let mut up = 0u64;
            for (name, data) in st.net.list() {
                let n = name.to_ascii_lowercase();
                if n.contains("loopback") || n == "lo" {
                    continue;
                }
                down += data.received();
                up += data.transmitted();
            }
            st.down_bps = down as f64 / elapsed;
            st.up_bps = up as f64 / elapsed;
            st.last = now;
        }

        let cpu = st.sys.global_cpu_usage();
        // 每核占用：调度器把负载摊到各核，单核打满时总占用看不出来
        let per_core: Vec<f64> = st
            .sys
            .cpus()
            .iter()
            .map(|c| (c.cpu_usage() as f64 * 10.0).round() / 10.0)
            .collect();
        json!({
            "cpu_pct": (cpu * 10.0).round() / 10.0,
            "per_core": per_core,
            "mem_used": st.sys.used_memory(),
            "mem_total": st.sys.total_memory(),
            "swap_used": st.sys.used_swap(),
            "swap_total": st.sys.total_swap(),
            "down_bps": st.down_bps.round(),
            "up_bps": st.up_bps.round(),
            "processes": st.proc_count,
            "uptime": sysinfo::System::uptime(),
        })
    })
    .unwrap_or(fallback)
}

/// 实时占用与网络速率。前端每 1~2 秒轮询。
#[tauri::command]
pub async fn sys_usage() -> serde_json::Value {
    tauri::async_runtime::spawn_blocking(collect_usage)
        .await
        .unwrap_or_else(|_| json!({ "cpu_pct": 0.0 }))
}

// ---------- 进程 ----------

#[derive(Serialize)]
pub struct ProcRow {
    pub pid: u32,
    /// 进程名
    pub name: String,
    /// 占系统总 CPU 的百分比（已按核心数折算）
    pub cpu: f64,
    /// 常驻内存字节
    pub mem: u64,
    /// 可执行文件路径，取不到为空
    pub exe: String,
    /// 命令行首段，取不到为空
    pub cmd: String,
}

fn collect_processes() -> Vec<ProcRow> {
    use sysinfo::{ProcessesToUpdate, System};

    let mut sys = System::new();
    // 首次刷新只建立基线，CPU 占用要第二次才有意义
    sys.refresh_processes(ProcessesToUpdate::All, true);
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let cores = sys.cpus().len().max(1) as f64;
    let mut rows: Vec<ProcRow> = sys
        .processes()
        .iter()
        .map(|(pid, p)| {
            let name = p.name().to_string_lossy().to_string();
            let exe = p
                .exe()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();
            let cmd = p
                .cmd()
                .first()
                .map(|c| c.to_string_lossy().to_string())
                .unwrap_or_default();
            ProcRow {
                pid: pid.as_u32(),
                name,
                cpu: ((p.cpu_usage() as f64 / cores) * 10.0).round() / 10.0,
                mem: p.memory(),
                exe,
                cmd,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.mem.cmp(&a.mem))
    });
    rows.truncate(120);
    rows
}

/// 进程列表，按 CPU 占用降序取前 120 个。
#[tauri::command]
pub async fn list_processes() -> Vec<ProcRow> {
    tauri::async_runtime::spawn_blocking(collect_processes)
        .await
        .unwrap_or_default()
}

/// 结束进程。系统关键进程或权限不足会返回错误，不会静默失败。
#[tauri::command]
pub async fn kill_pid(pid: u32) -> Result<(), String> {
    if pid == 0 || pid == 4 {
        return Err("系统进程不能结束".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        use sysinfo::{Pid, ProcessesToUpdate, System};
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let key = Pid::from_u32(pid);
        let Some(p) = sys.process(key) else {
            return Err("进程已退出".to_string());
        };
        if p.kill() {
            Ok(())
        } else {
            Err("结束失败：权限不足或进程受保护".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- 网络适配器 / 诊断 ----------

fn ps_run(script: &str) -> String {
    use std::os::windows::process::CommandExt;
    let full = format!("[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; {}", script);
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &full])
        .creation_flags(0x0800_0000)
        .output();
    match out {
        Ok(o) => {
            let bytes = o.stdout;
            let bytes = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(&bytes);
            let utf16 =
                bytes.len() >= 4 && bytes.len() % 2 == 0 && bytes[1] == 0 && bytes[3] == 0;
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
    if let Some(start) = text.find(['{', '[']) {
        if let Ok(v) = serde_json::from_str(&text[start..]) {
            return Some(v);
        }
    }
    // 单元素数组经管道会被展开成裸字符串（"1.2.3.4"），这里补一次整体解析
    serde_json::from_str(&text).ok()
}

fn as_array(v: Option<serde_json::Value>) -> Vec<serde_json::Value> {
    match v {
        Some(serde_json::Value::Array(a)) => a,
        Some(serde_json::Value::Null) | None => vec![],
        Some(x) => vec![x],
    }
}

fn jtext(v: &serde_json::Value, key: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// 已启用的网络适配器：名称、IPv4、网关、DNS、MAC、速率。
#[tauri::command]
pub async fn net_adapters() -> Vec<serde_json::Value> {
    tauri::async_runtime::spawn_blocking(|| {
        let script = r#"
$out = @()
$ips = @()
try { $ips = @(Get-NetIPConfiguration | Where-Object { $_.NetAdapter.Status -eq 'Up' }) } catch {}
$macs = @{}
try {
  Get-NetAdapter | Where-Object { $_.Status -eq 'Up' } | ForEach-Object {
    $macs[$_.InterfaceAlias] = @{ mac = [string]$_.MacAddress; speed = [string]$_.LinkSpeed }
  }
} catch {}
foreach ($c in $ips) {
  $alias = [string]$c.InterfaceAlias
  $v4 = ''
  if ($c.IPv4Address) { $v4 = ($c.IPv4Address.IPAddress -join ', ') }
  $gw = ''
  if ($c.IPv4DefaultGateway) { $gw = ($c.IPv4DefaultGateway.NextHop -join ', ') }
  $dns = ''
  if ($c.DNSServer) {
    $dns = (($c.DNSServer | Where-Object { $_.AddressFamily -eq 2 } | ForEach-Object { $_.ServerAddresses }) -join ', ')
  }
  $m = $macs[$alias]
  $out += @{ alias = $alias; ipv4 = $v4; gateway = $gw; dns = $dns; mac = [string]$m.mac; speed = [string]$m.speed }
}
if ($out.Count -eq 0) {
  try {
    Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } | ForEach-Object {
      $out += @{ alias = [string]$_.InterfaceAlias; ipv4 = [string]$_.IPAddress; gateway = ''; dns = ''; mac = ''; speed = '' }
    }
  } catch {}
}
        ConvertTo-Json -InputObject @($out) -Compress -Depth 4
"#;
        let parsed = ps_json(script);
        as_array(parsed)
            .into_iter()
            .map(|a| {
                json!({
                    "alias": jtext(&a, "alias"),
                    "ipv4": jtext(&a, "ipv4"),
                    "gateway": jtext(&a, "gateway"),
                    "dns": jtext(&a, "dns"),
                    "mac": jtext(&a, "mac"),
                    "speed": jtext(&a, "speed"),
                })
            })
            .filter(|v| !v["alias"].as_str().unwrap_or("").is_empty())
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// 对主机发 4 个 ICMP 包，返回丢包率与延迟。
#[tauri::command]
pub async fn ping_host(host: String) -> Result<serde_json::Value, String> {
    let host = host.trim().to_string();
    if host.is_empty() {
        return Err("请填写域名或 IP".into());
    }
    // 只允许域名/IP 字符，避免拼进 PowerShell 脚本被注入
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'))
    {
        return Err("包含非法字符".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let script = format!(
            r#"
$r = @()
try {{ $r = @(Test-Connection -ComputerName '{host}' -Count 4 -ErrorAction SilentlyContinue) }} catch {{}}
if ($r.Count -eq 0) {{
  @{{ ok = $false; sent = 4; lost = 4; min = $null; avg = $null; max = $null }} | ConvertTo-Json -Compress
}} else {{
  $t = @($r | ForEach-Object {{ [double]$_.ResponseTime }})
  $m = $t | Measure-Object -Minimum -Maximum -Average
  @{{
    ok = $true
    sent = 4
    lost = 4 - $r.Count
    min = [math]::Round($m.Minimum, 1)
    avg = [math]::Round($m.Average, 1)
    max = [math]::Round($m.Maximum, 1)
  }} | ConvertTo-Json -Compress
}}
"#
        );
        match ps_json(&script) {
            Some(v) => {
                let lost = v.get("lost").and_then(|x| x.as_u64()).unwrap_or(4);
                Ok(json!({
                    "ok": v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false),
                    "sent": v.get("sent").and_then(|x| x.as_u64()).unwrap_or(4),
                    "lost": lost,
                    "loss_pct": (lost as f64 / 4.0 * 100.0).round(),
                    "min": v.get("min").and_then(|x| x.as_f64()),
                    "avg": v.get("avg").and_then(|x| x.as_f64()),
                    "max": v.get("max").and_then(|x| x.as_f64()),
                }))
            }
            None => Err("Ping 无响应".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 域名解析，返回 A / AAAA 记录。
#[tauri::command]
pub async fn dns_lookup(host: String) -> Result<Vec<String>, String> {
    let host = host.trim().to_string();
    if host.is_empty() {
        return Err("请填写域名".into());
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err("包含非法字符".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let script = format!(
            r#"
$out = @()
try {{
  $out = @(Resolve-DnsName -Name '{host}' -Type A -ErrorAction Stop | Where-Object {{ $_.IPAddress }} | ForEach-Object {{ [string]$_.IPAddress }})
}} catch {{}}
if ($out.Count -eq 0) {{
  try {{
    $out = @(Resolve-DnsName -Name '{host}' -Type AAAA -ErrorAction Stop | Where-Object {{ $_.IPAddress }} | ForEach-Object {{ [string]$_.IPAddress }})
  }} catch {{}}
}}
ConvertTo-Json -InputObject @($out) -Compress
"#
        );
        match ps_json(&script) {
            Some(serde_json::Value::String(ip)) => Ok(vec![ip]),
            Some(v) => {
                let list: Vec<String> = as_array(Some(v))
                    .into_iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .filter(|s| !s.is_empty())
                    .collect();
                if list.is_empty() {
                    Err("解析失败：没有返回地址".to_string())
                } else {
                    Ok(list)
                }
            }
            None => Err("解析失败：域名不存在或无响应".to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 清理本机 DNS 缓存。
#[tauri::command]
pub async fn flush_dns() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("ipconfig")
            .arg("/flushdns")
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|e| format!("无法执行 ipconfig: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if out.status.success() && (text.contains("成功") || text.to_ascii_lowercase().contains("success")) {
            Ok("DNS 缓存已清理".to_string())
        } else {
            Err("清理失败，可能需要管理员权限".to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::{as_array, collect_processes, collect_usage, ps_json};
    use serde_json::json;

    #[test]
    fn as_array_wraps_single_and_nulls() {
        assert_eq!(as_array(Some(json!({"a": 1}))).len(), 1);
        assert_eq!(as_array(Some(serde_json::Value::Null)).len(), 0);
        assert_eq!(as_array(None).len(), 0);
        assert_eq!(as_array(Some(json!([1, 2, 3]))).len(), 3);
    }

    #[test]
    fn host_allows_only_safe_chars() {
        let ok = |s: &str| s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'));
        assert!(ok("www.baidu.com"));
        assert!(ok("1.1.1.1"));
        assert!(ok("2606:4700::1111"));
        assert!(!ok("a; rm -rf"));
        assert!(!ok("a'b"));
    }

    /// 现场探针：确认采集链路在本机可用（cargo test live_ -- --ignored --nocapture）
    #[test]
    #[ignore]
    fn live_usage_and_processes() {
        let u = collect_usage();
        println!("usage = {u}");
        assert!(u["mem_total"].as_u64().unwrap_or(0) > 0);
        let p = collect_processes();
        println!("processes = {}", p.len());
        assert!(!p.is_empty());
    }

    #[test]
    #[ignore]
    fn live_net_dns_ping() {
        let script = r#"
ConvertTo-Json -InputObject @(@{ alias = 'x'; ipv4 = '1.2.3.4' }) -Compress
"#;
        let v = ps_json(script);
        println!("adapters = {v:?}");
        assert!(v.is_some());

        let dns = r#"ConvertTo-Json -InputObject @(@('1.2.3.4')) -Compress"#;
        println!("dns = {:?}", ps_json(dns));
    }
}
