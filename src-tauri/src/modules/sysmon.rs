//! 系统监控：实时占用、网络速率、进程管理、网络诊断

use serde::Serialize;
use serde_json::json;
use std::os::windows::process::CommandExt;
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

/// 结束进程树：先递归收集子孙进程，再自底向上结束。
/// 只结束当前进程的子树，不会碰父进程，避免误杀启动器/explorer。
#[tauri::command]
pub async fn kill_process_tree(pid: u32) -> Result<u32, String> {
    if pid == 0 || pid == 4 {
        return Err("系统进程不能结束".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        use sysinfo::{Pid, ProcessesToUpdate, System};
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let root = Pid::from_u32(pid);
        if sys.process(root).is_none() {
            return Err("进程已退出".to_string());
        }

        // 收集整棵子树（含自己）
        let mut tree: Vec<Pid> = vec![root];
        let mut i = 0;
        while i < tree.len() {
            let parent = tree[i];
            for (child, p) in sys.processes() {
                if p.parent() == Some(parent) && !tree.contains(child) {
                    tree.push(*child);
                }
            }
            i += 1;
            // 防御性上限：异常进程树不该有上千个后代
            if tree.len() > 2000 {
                break;
            }
        }

        // 先杀叶子，再杀父级，减少「父进程退出后子进程被重新拉起」的概率
        let mut killed = 0u32;
        for p in tree.iter().rev() {
            if let Some(proc_item) = sys.process(*p) {
                if proc_item.kill() {
                    killed += 1;
                }
            }
        }
        if killed == 0 {
            return Err("结束失败：权限不足或进程受保护".to_string());
        }
        Ok(killed)
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

/// 解析 tracert 输出为逐跳数据。纯函数，便于测试。
/// 注意：真实输出里数字与单位是分开的（`2 ms`，不是 `2ms`），
/// 所以遇到 `ms`/`MS` 时要回看前一个词才是延迟值。
fn parse_tracert(text: &str) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        // 必须先看数字，注释行与标题行没有行号
        let Some((hop, rest)) = t.split_once(char::is_whitespace) else { continue };
        let Ok(hop_num) = hop.parse::<u32>() else { continue };
        let rest = rest.trim();
        if rest.is_empty() {
            continue;
        }

        let parts: Vec<&str> = rest.split_whitespace().collect();
        let mut delays: Vec<Option<f64>> = Vec::new();
        let mut addr = String::new();
        for (i, part) in parts.iter().enumerate() {
            let low = part.to_ascii_lowercase();
            if low == "ms" {
                // 延迟值在单位前一个词
                if i > 0 {
                    let prev = parts[i - 1].trim_start_matches('<');
                    if let Ok(v) = prev.parse::<f64>() {
                        delays.push(Some(v));
                        continue;
                    }
                }
                delays.push(None);
            } else if *part == "*" {
                delays.push(None);
            } else if low.ends_with("ms") {
                // 兼容 "1ms" 紧贴写法
                let num = low
                    .trim_end_matches("ms")
                    .trim_start_matches('<')
                    .trim()
                    .parse::<f64>()
                    .ok();
                delays.push(num);
            } else if !part.is_empty()
                && part.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ':')
                && part.contains('.')
            {
                // 纯数字带点的是 IP；跳数本身已在前面取走
                addr = part.to_string();
            }
        }

        // 只保留前 3 个延迟值（tracert 默认发 3 个包）
        delays.truncate(3);
        let timeout = delays.iter().all(|d| d.is_none()) && addr.is_empty();
        let vals: Vec<f64> = delays.iter().flatten().copied().collect();
        let avg_ms = if vals.is_empty() {
            0.0
        } else {
            (vals.iter().sum::<f64>() / vals.len() as f64 * 10.0).round() / 10.0
        };
        out.push(json!({
            "hop": hop_num,
            "addr": addr,
            "delays": delays,
            "timeout": timeout,
            "avg_ms": avg_ms,
        }));
    }
    out
}

/// 路由追踪。host 做字符白名单校验，防止拼进命令被注入。
#[tauri::command]
pub async fn trace_route(host: String, max_hops: u64) -> Result<Vec<serde_json::Value>, String> {
    let host = host.trim().to_string();
    if host.is_empty() {
        return Err("请填写域名或 IP".into());
    }
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'))
    {
        return Err("包含非法字符".into());
    }
    let hops = max_hops.clamp(1, 30);
    tauri::async_runtime::spawn_blocking(move || {
        use std::os::windows::process::CommandExt;
        // -d 不解析主机名、-w 800ms 超时、-h 跳数上限，整体更快
        let out = std::process::Command::new("tracert")
            .args(["-d", "-w", "800", "-h", &hops.to_string(), &host])
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|e| format!("无法执行 tracert: {e}"))?;
        let bytes = out.stdout;
        let bytes = if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
            &bytes[2..]
        } else {
            &bytes[..]
        };
        // 中文系统 tracert 输出是 GBK，先试 UTF-8 再回落
        let text = match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => {
                let (t, _, _) = encoding_rs::GBK.decode(bytes);
                t.into_owned()
            }
        };
        let list = parse_tracert(&text);
        if list.is_empty() {
            return Err("没有解析到任何跳。目标可能不可达。".to_string());
        }
        Ok(list)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 局域网设备扫描结果
#[derive(Serialize)]
pub struct LanDevice {
    pub ip: String,
    pub mac: String,
    /// 解析到的主机名，取不到为空
    pub hostname: String,
    /// 是否本机
    pub is_self: bool,
}

/// 解析 arp -a 输出。纯函数，便于测试。
/// 只保留合法 IPv4 + 有效 MAC，剔除广播/组播/链路本地（169.254.*）。
fn parse_arp(text: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let ip = parts[0];
        let mac = parts[1].replace('-', ":");
        if !ip.chars().all(|c| c.is_ascii_digit() || c == '.') || ip.matches('.').count() != 3 {
            continue;
        }
        // MAC 必须是 6 段十六进制
        let mac_parts: Vec<&str> = mac.split(':').collect();
        if mac_parts.len() != 6 || !mac_parts.iter().all(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_hexdigit())) {
            continue;
        }
        if mac == "ff:ff:ff:ff:ff:ff" {
            continue;
        }
        if ip.ends_with(".255") || ip.starts_with("224.") || ip.starts_with("239.") || ip.ends_with(".0") {
            continue;
        }
        // 169.254.x.x 是链路本地地址：网卡没拿到 DHCP 会堆积几百条陈旧记录，
        // 不代表有真实设备在线，必须剔除，否则列表会被垃圾数据刷满。
        if ip.starts_with("169.254.") {
            continue;
        }
        rows.push((ip.to_string(), mac.to_ascii_uppercase()));
    }
    rows.sort_by(|a, b| {
        // 按 IP 数值排序，让 192.168.1.2 排在 192.168.1.10 前面
        let key = |s: &str| -> u32 {
            let mut v = 0u32;
            for p in s.split('.') {
                v = v.saturating_mul(256).saturating_add(p.parse::<u32>().unwrap_or(0));
            }
            v
        };
        key(&a.0).cmp(&key(&b.0))
    });
    rows
}

/// 扫描本局域网在线设备（ARP 表 + 并发 ping 激活）。
/// 只读取本机 ARP 缓存，不主动对网段做暴力扫描，避免触发防火墙告警。
#[tauri::command]
pub async fn scan_lan_devices() -> Vec<LanDevice> {
    tauri::async_runtime::spawn_blocking(|| {
        let read_arp = || {
            let out = std::process::Command::new("arp")
                .args(["-a"])
                .creation_flags(0x0800_0000)
                .output();
            let Ok(o) = out else { return Vec::new() };
            let bytes = o.stdout;
            let text = match std::str::from_utf8(&bytes) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    let (t, _, _) = encoding_rs::GBK.decode(&bytes);
                    t.into_owned()
                }
            };
            parse_arp(&text)
        };

        let locals = local_ipv4_list();
        let self_ip = locals.first().cloned().unwrap_or_default();
        let mut rows = read_arp();
        // ARP 表可能是空的（刚开机），先 ping 网关激活一下再读一次
        if rows.is_empty() {
            let gateway = self_ip
                .rsplit_once('.')
                .map(|(head, _)| format!("{head}.1"))
                .unwrap_or_else(|| "192.168.1.1".to_string());
            let _ = std::process::Command::new("ping")
                .args(["-n", "1", "-w", "600", &gateway])
                .creation_flags(0x0800_0000)
                .output();
            rows = read_arp();
        }

        same_subnet(rows, &locals)
            .into_iter()
            .map(|(ip, mac)| {
                let is_self = locals.iter().any(|l| l == &ip);
                LanDevice {
                    hostname: if is_self { "本机".to_string() } else { String::new() },
                    ip,
                    mac,
                    is_self,
                }
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// 取本机所有 IPv4 地址（含多网卡）。
fn local_ipv4_list() -> Vec<String> {
    let mut out = Vec::new();
    // 主出口 IP：连一个外部地址不会真的发包，只让系统选路
    use std::net::UdpSocket;
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                out.push(addr.ip().to_string());
            }
        }
    }
    // 其余网卡从 ipconfig 补全，多网卡时才能覆盖全部网段
    let out_cmd = std::process::Command::new("ipconfig")
        .creation_flags(0x0800_0000)
        .output();
    if let Ok(o) = out_cmd {
        let bytes = o.stdout;
        let text = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            Err(_) => {
                let (t, _, _) = encoding_rs::GBK.decode(&bytes);
                t.into_owned()
            }
        };
        for line in text.lines() {
            if !line.contains("IPv4") {
                continue;
            }
            if let Some(idx) = line.find(':') {
                let candidate = line[idx + 1..].trim();
                if candidate.matches('.').count() == 3
                    && candidate.chars().all(|c| c.is_ascii_digit() || c == '.')
                    && !candidate.starts_with("127.")
                    && !candidate.starts_with("169.254.")
                {
                    out.push(candidate.to_string());
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 把 IP 归到 /24 网段键（前三段）
fn subnet_of(ip: &str) -> String {
    let mut it = ip.split('.');
    match (it.next(), it.next(), it.next()) {
        (Some(a), Some(b), Some(c)) => format!("{a}.{b}.{c}"),
        _ => String::new(),
    }
}

/// 只保留与本机主出口处在同一 /24 网段的 ARP 条目。
/// 实测本机 ARP 表会跨 8 个网段、580+ 条记录（代理软件 / 虚拟机造成），
/// 全量展示等于刷屏，而那些网段并不是用户理解的「局域网」。
/// 多网卡时优先用 primary（主出口 IP），取不到才退回全部网卡。
fn same_subnet(rows: Vec<(String, String)>, locals: &[String]) -> Vec<(String, String)> {
    let nets: Vec<String> = locals
        .iter()
        .map(|ip| subnet_of(ip))
        .filter(|s| !s.is_empty())
        .collect();
    if nets.is_empty() {
        return rows;
    }
    // 主出口网段优先：只保留它，避免把代理虚拟网段（192.168.9.x 之类）当作局域网
    let primary = nets.first().cloned().unwrap_or_default();
    let kept: Vec<(String, String)> = rows
        .iter()
        .filter(|(ip, _)| subnet_of(ip) == primary)
        .cloned()
        .collect();
    // 主网段一条都没有时说明选路判断失败，退回全部本机网段
    if kept.is_empty() {
        return rows
            .into_iter()
            .filter(|(ip, _)| nets.contains(&subnet_of(ip)))
            .collect();
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::{as_array, collect_processes, collect_usage, parse_tracert, ps_json};
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

    #[test]
    fn tracert_output_is_parsed() {
        // 真实 tracert -d 输出片段（英文系统）
        let text = r#"
Tracing route to 8.8.8.8 over a maximum of 30 hops

  1     2 ms     1 ms     1 ms  192.168.1.1
  2     *        *        *     Request timed out.
  3    12 ms    11 ms    13 ms  10.10.0.1
  4     8 ms     9 ms     8 ms  8.8.8.8

Trace complete.
"#;
        let hops = parse_tracert(text);
        assert_eq!(hops.len(), 4);
        assert_eq!(hops[0]["hop"], 1);
        assert_eq!(hops[0]["addr"], "192.168.1.1");
        assert_eq!(hops[0]["avg_ms"], 1.3);
        // 超时跳没有地址，avg 为 0
        assert_eq!(hops[1]["addr"], "");
        assert_eq!(hops[1]["avg_ms"], 0.0);
        assert_eq!(hops[1]["timeout"], true);
        assert_eq!(hops[3]["addr"], "8.8.8.8");
    }

    #[test]
    fn tracert_sub_ms_delay_parsed() {
        // "<1 ms" 形式要能解析
        let text = "  1    <1 ms    <1 ms    <1 ms  192.168.0.1\n";
        let hops = parse_tracert(text);
        assert_eq!(hops.len(), 1);
        assert_eq!(hops[0]["addr"], "192.168.0.1");
        assert_eq!(hops[0]["avg_ms"], 1.0);
    }

    #[test]
    fn tracert_garbage_yields_nothing() {
        // 空输出 / 报错文本不能崩，也不能造出假跳
        assert!(parse_tracert("").is_empty());
        assert!(parse_tracert("Unable to resolve target system name.").is_empty());
        assert!(parse_tracert("Trace complete.").is_empty());
    }

    #[test]
    fn arp_parsing_filters_junk() {
        // 真实 arp -a 输出（含网关、动态/静态条目、广播行）
        let text = r#"
Interface: 192.168.1.100 --- 0x5
  Internet Address      Physical Address      Type
  192.168.1.1           00-11-22-33-44-55     dynamic
  192.168.1.10          00-11-22-33-44-66     dynamic
  192.168.1.255         ff-ff-ff-ff-ff-ff     static
  224.0.0.22            01-00-5e-00-00-16     static
  169.254.24.21         00-e0-97-27-00-48     dynamic
  239.255.255.250       01-00-5e-7f-ff-fa     static
"#;
        let rows = super::parse_arp(text);
        // 只应剩下 192.168.1.1 与 192.168.1.10，且按 IP 数值升序
        assert_eq!(rows.len(), 2, "实际解析出 {:?}", rows);
        assert_eq!(rows[0].0, "192.168.1.1");
        assert_eq!(rows[0].1, "00:11:22:33:44:55");
        assert_eq!(rows[1].0, "192.168.1.10");
    }

    #[test]
    fn arp_link_local_is_dropped() {
        // 网卡没拿到 DHCP 时会堆积几百条 169.254，必须全部剔除
        let text = "  169.254.1.1   AA-BB-CC-DD-EE-FF   dynamic\n  169.254.2.2   AA-BB-CC-DD-EE-01   dynamic\n";
        assert!(super::parse_arp(text).is_empty());
    }

    #[test]
    fn arp_sorts_numerically() {
        // 字符串排序会把 .10 排到 .2 前面，必须按数值
        let text = "  10.0.0.10   AA-BB-CC-DD-EE-01   dynamic\n  10.0.0.2   AA-BB-CC-DD-EE-02   dynamic\n";
        let rows = super::parse_arp(text);
        assert_eq!(rows[0].0, "10.0.0.2");
        assert_eq!(rows[1].0, "10.0.0.10");
    }

    #[test]
    fn subnet_filter_keeps_only_local_network() {
        // 本机 ARP 表实测跨 8 个网段、580+ 条，全量展示等于刷屏
        let rows = vec![
            ("192.168.8.2".to_string(), "AA:AA:AA:AA:AA:01".to_string()),
            ("192.168.8.3".to_string(), "AA:AA:AA:AA:AA:02".to_string()),
            ("192.168.9.5".to_string(), "AA:AA:AA:AA:AA:03".to_string()),
            ("10.0.0.7".to_string(), "AA:AA:AA:AA:AA:04".to_string()),
        ];
        let kept = super::same_subnet(rows, &["192.168.8.100".to_string()]);
        assert_eq!(kept.len(), 2, "只应保留 192.168.8.x");
        assert!(kept.iter().all(|(ip, _)| ip.starts_with("192.168.8.")));
    }

    #[test]
    fn subnet_filter_prefers_primary_network() {
        // 多网卡时只取主出口网段，否则会把代理虚拟网段当成局域网
        let rows = vec![
            ("192.168.9.5".to_string(), "AA:AA:AA:AA:AA:03".to_string()),
            ("192.168.8.2".to_string(), "AA:AA:AA:AA:AA:01".to_string()),
        ];
        let kept = super::same_subnet(
            rows,
            &["192.168.8.100".to_string(), "192.168.9.100".to_string()],
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0, "192.168.8.2");
    }

    #[test]
    fn subnet_filter_falls_back_when_primary_empty() {
        // 主网段没有条目（选路判断失败）时应退回全部本机网段，而不是返回空
        let rows = vec![("192.168.9.5".to_string(), "AA:AA:AA:AA:AA:03".to_string())];
        let kept = super::same_subnet(
            rows,
            &["192.168.8.100".to_string(), "192.168.9.100".to_string()],
        );
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn subnet_filter_without_local_ip_keeps_all() {
        // 拿不到本机 IP 时不能把结果全丢掉，否则界面永远空白
        let rows = vec![("1.2.3.4".to_string(), "AA:AA:AA:AA:AA:01".to_string())];
        assert_eq!(super::same_subnet(rows.clone(), &[]).len(), 1);
        assert_eq!(super::same_subnet(rows, &["".to_string()]).len(), 1);
    }

    #[test]
    fn subnet_key_takes_three_octets() {
        assert_eq!(super::subnet_of("192.168.8.123"), "192.168.8");
        assert_eq!(super::subnet_of("10.0.0.1"), "10.0.0");
        assert_eq!(super::subnet_of(""), "");
        // 段数不足时不该 panic
        assert_eq!(super::subnet_of("1.2"), "");
    }

    /// 现场探针：真跑一次 tracert + 局域网扫描。
    /// `cargo test --lib live_trace_and_lan -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_trace_and_lan() {
        let hops = tauri::async_runtime::block_on(super::trace_route("223.5.5.5".into(), 12))
            .expect("tracert 应该能执行");
        println!("hops={}", hops.len());
        for h in hops.iter().take(10) {
            println!(
                "  {}  {}  avg={}ms  timeout={}",
                h["hop"], h["addr"], h["avg_ms"], h["timeout"]
            );
        }
        assert!(!hops.is_empty(), "应该至少解析到一跳");

        let lan = tauri::async_runtime::block_on(super::scan_lan_devices());
        println!("lan devices={}", lan.len());
        for d in lan.iter().take(10) {
            println!("  {}  {}  {}", d.ip, d.mac, d.hostname);
        }
    }
}
