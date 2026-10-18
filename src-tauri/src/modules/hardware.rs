//! 硬件信息、CPU 烤机、磁盘测速

use serde::Serialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::Emitter;
use winreg::enums::HKEY_LOCAL_MACHINE;
use winreg::RegKey;

static BURN_RUN: AtomicBool = AtomicBool::new(false);
static BURN_WORKERS: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());
static BURN_MONITOR: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

#[derive(Serialize)]
pub struct DiskSpeed {
    pub read_mbps: f64,
    pub write_mbps: f64,
}

// ---------- PowerShell WMI 辅助 ----------

/// PowerShell 管道在无控制台时可能是 UTF-16 LE，控制台下则是 UTF-8（带或不带 BOM）
fn decode_ps_stdout(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xFF, 0xFE]).unwrap_or(bytes);
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

fn ps_json(script: &str) -> Option<serde_json::Value> {
    let full = format!("[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; {}", script);
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &full]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    let text = decode_ps_stdout(&out.stdout);
    let start = text.find(['{', '['])?;
    serde_json::from_str(&text[start..]).ok()
}

/// ConvertTo-Json 单元素时输出对象而非数组 → 统一为数组。null 视为空。
fn as_array(v: Option<serde_json::Value>) -> Vec<serde_json::Value> {
    match v {
        Some(serde_json::Value::Array(a)) => a,
        Some(serde_json::Value::Null) | None => vec![],
        Some(x) => vec![x],
    }
}

fn json_text(v: &serde_json::Value, key: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn json_u64(v: &serde_json::Value, key: &str) -> u64 {
    match v.get(key) {
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0).max(0.0) as u64,
        Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0).max(0.0) as u64,
        _ => 0,
    }
}

fn jstr(v: &serde_json::Value, key: &str, default: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(default)
        .to_string()
}

fn jnum(v: &serde_json::Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(default)
}

/// SMBIOS 内存类型码 → DDR 代际。未知返回空串，前端不展示。
fn mem_type_name(code: u64) -> &'static str {
    match code {
        20 => "DDR",
        21 => "DDR2",
        24 => "DDR3",
        26 => "DDR4",
        34 => "DDR5",
        _ => "",
    }
}

/// Win32_PhysicalMemory.FormFactor。0/未知返回空串。
fn form_factor_name(code: u64) -> &'static str {
    match code {
        8 => "DIMM",
        9 => "FB-DIMM",
        12 => "SO-DIMM",
        _ => "",
    }
}

/// Win32_Processor.Architecture。调用方须先确认字段存在，0 是合法的 x86。
fn cpu_arch_name(code: u64) -> &'static str {
    match code {
        0 => "x86",
        5 => "ARM",
        9 => "x64",
        12 => "ARM64",
        _ => "",
    }
}

fn cpu_vendor_name(raw: &str) -> String {
    match raw.trim() {
        "GenuineIntel" => "Intel".to_string(),
        "AuthenticAMD" => "AMD".to_string(),
        other => other.to_string(),
    }
}

fn json_bool(v: &serde_json::Value, key: &str) -> Option<bool> {
    match v.get(key) {
        Some(serde_json::Value::Bool(b)) => Some(*b),
        Some(serde_json::Value::Number(n)) => Some(n.as_f64().unwrap_or(0.0) != 0.0),
        Some(serde_json::Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// 远程/虚拟显示设备没有真实显存，不能和独显混在一起看起来像「没读到」。
fn is_virtual_gpu(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "oray",
        "idd",
        "virtual",
        "asklink",
        "gameviewer",
        "indirect",
        "basic display",
        "remote",
        "spacedesk",
        "parsec",
        "sunshine",
        "iddcx",
        "hyper-v",
    ]
    .iter()
    .any(|k| n.contains(k))
}

/// WMI 日期：`yyyy-MM-dd`、`/Date(毫秒)/`、DMTF `yyyyMMdd...`
fn parse_wmi_date(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 10 && t.as_bytes().get(4) == Some(&b'-') && t.as_bytes().get(7) == Some(&b'-') {
        return t.chars().take(10).collect();
    }
    if let Some(start) = t.find("Date(") {
        let num: String = t[start + 5..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(ms) = num.parse::<i64>() {
            return unix_days_to_ymd(ms.div_euclid(86_400_000));
        }
    }
    if t.len() >= 8 && t.as_bytes()[..8].iter().all(|b| b.is_ascii_digit()) {
        return format!("{}-{}-{}", &t[0..4], &t[4..6], &t[6..8]);
    }
    String::new()
}

/// Unix 天数（1970-01-01 起）→ 公历。Howard Hinnant civil_from_days。
fn unix_days_to_ymd(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Storage 枚举在部分系统上被 ConvertTo-Json 写成数字，界面就会空白。
fn map_storage_enum(kind: &str, raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    if raw.chars().any(|c| !c.is_ascii_digit()) {
        return raw.to_string();
    }
    let n: i64 = raw.parse().unwrap_or(-1);
    let mapped: Option<&str> = match kind {
        "bus" => match n {
            3 => Some("ATA"),
            7 => Some("USB"),
            8 => Some("RAID"),
            10 => Some("SAS"),
            11 => Some("SATA"),
            17 => Some("NVMe"),
            _ => None,
        },
        "media" => match n {
            3 => Some("HDD"),
            4 => Some("SSD"),
            5 => Some("SCM"),
            _ => None,
        },
        "health" => match n {
            0 => Some("Healthy"),
            1 => Some("Warning"),
            2 => Some("Unhealthy"),
            _ => None,
        },
        _ => None,
    };
    mapped.unwrap_or(raw).to_string()
}

fn reg_qword(k: &RegKey, name: &str) -> u64 {
    k.get_value::<u64, _>(name)
        .or_else(|_| k.get_value::<u32, _>(name).map(|v| v as u64))
        .unwrap_or(0)
}

/// 显示适配器类里的专用显存（字节→MB）和驱动版本。同名取较大显存，避开历史残留里的 0。
fn registry_gpu_extra() -> std::collections::HashMap<String, (u64, String, String)> {
    let mut map = std::collections::HashMap::<String, (u64, String, String)>::new();
    let base = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(base) else {
        return map;
    };
    for sub in k.enum_keys().flatten() {
        if !sub.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(sk) = k.open_subkey(&sub) else { continue };
        let Ok(desc) = sk.get_value::<String, _>("DriverDesc") else { continue };
        let desc = desc.trim().to_string();
        if desc.is_empty() {
            continue;
        }
        let mb = reg_qword(&sk, "HardwareInformation.qwMemorySize") / 1_048_576;
        let driver = sk.get_value::<String, _>("DriverVersion").unwrap_or_default();
        map.entry(desc.to_ascii_lowercase())
            .and_modify(|(old_mb, old_drv, old_name)| {
                if mb > *old_mb {
                    *old_mb = mb;
                    *old_name = desc.clone();
                }
                if old_drv.is_empty() && !driver.is_empty() {
                    *old_drv = driver.clone();
                }
            })
            .or_insert((mb, driver, desc));
    }
    map
}

fn cpu_name_registry() -> String {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0")
        .ok()
        .and_then(|k| k.get_value::<String, _>("ProcessorNameString").ok())
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// 一次 CIM 查询拿齐 CPU / 主板 / BIOS / 显卡 / 磁盘 / 内存 / 系统。
/// 注册表 BIOS 键在不少机器上是空的，不能只靠它。
/// 虚拟化字段保持 $null，不能强转 bool，否则「读不到」会变成「未开启」。
fn cim_snapshot() -> Option<serde_json::Value> {
    ps_json(
        r#"
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$bb = Get-CimInstance Win32_BaseBoard | Select-Object -First 1
$bios = Get-CimInstance Win32_BIOS | Select-Object -First 1
$os = Get-CimInstance Win32_OperatingSystem | Select-Object -First 1
$date = ''
if ($bios.ReleaseDate) { try { $date = Get-Date $bios.ReleaseDate -Format 'yyyy-MM-dd' } catch { $date = '' } }
$virt = $null
if ($null -ne $cpu.VirtualizationFirmwareEnabled) { $virt = [bool]$cpu.VirtualizationFirmwareEnabled }
$phys = @()
try { $phys = @(Get-PhysicalDisk | Select-Object FriendlyName,MediaType,BusType,HealthStatus,Size,SerialNumber) } catch {}
$gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object {
  $dd = ''
  if ($_.DriverDate) { try { $dd = Get-Date $_.DriverDate -Format 'yyyy-MM-dd' } catch { $dd = [string]$_.DriverDate } }
  @{
    Name = [string]$_.Name
    AdapterRAM = $_.AdapterRAM
    DriverVersion = [string]$_.DriverVersion
    DriverDate = $dd
    Width = $_.CurrentHorizontalResolution
    Height = $_.CurrentVerticalResolution
    Refresh = $_.CurrentRefreshRate
    Vendor = [string]$_.AdapterCompatibility
  }
})
@{
  cpu = @{
    name = [string]$cpu.Name
    cores = $cpu.NumberOfCores
    threads = $cpu.NumberOfLogicalProcessors
    mhz = $cpu.MaxClockSpeed
    current_mhz = $cpu.CurrentClockSpeed
    l2_kb = $cpu.L2CacheSize
    l3_kb = $cpu.L3CacheSize
    socket = [string]$cpu.SocketDesignation
    vendor = [string]$cpu.Manufacturer
    arch = $cpu.Architecture
    virtualization = $virt
  }
  board = @{ manufacturer = [string]$bb.Manufacturer; product = [string]$bb.Product; version = [string]$bb.Version; serial = [string]$bb.SerialNumber }
  bios = @{ vendor = [string]$bios.Manufacturer; version = [string]$bios.SMBIOSBIOSVersion; date = [string]$date }
  os = @{ caption = [string]$os.Caption; version = [string]$os.Version; build = [string]$os.BuildNumber; arch = [string]$os.OSArchitecture }
  gpus = $gpus
  disks = @(Get-CimInstance Win32_DiskDrive | Select-Object Model,Size,InterfaceType,SerialNumber,FirmwareRevision)
  phys = $phys
  mem = @(Get-CimInstance Win32_PhysicalMemory | Select-Object Manufacturer,DeviceLocator,BankLabel,PartNumber,Speed,ConfiguredClockSpeed,Capacity,SMBIOSMemoryType,FormFactor,DataWidth)
} | ConvertTo-Json -Compress -Depth 6
"#,
    )
}

fn volumes_info() -> Vec<serde_json::Value> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| d.total_space() > 0 && !d.mount_point().as_os_str().is_empty())
        .map(|d| {
            json!({
                "mount": d.mount_point().to_string_lossy(),
                "total": d.total_space(),
                "free": d.available_space(),
            })
        })
        .collect()
}

fn nonempty(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && !t.eq_ignore_ascii_case("default string") && !t.eq_ignore_ascii_case("to be filled by o.e.m.")
}

/// AdapterRAM（字节）→ MB。WMI 用 uint32，大于 4GB 的显卡常返回 -1 或溢出值，此时记 0（未知）。
fn vram_mb(raw: f64) -> u64 {
    const MAX_BYTES: f64 = 64.0 * 1024.0 * 1024.0 * 1024.0;
    if raw <= 0.0 || raw > MAX_BYTES {
        return 0;
    }
    (raw / 1_048_576.0).round() as u64
}

/// AdapterRAM 在 4GB 附近饱和时不可信（uint32），有注册表专用显存就用注册表。
fn pick_vram_mb(adapter_ram: f64, registry_mb: u64) -> u64 {
    if registry_mb > 0 {
        return registry_mb;
    }
    vram_mb(adapter_ram)
}

fn gpu_value(
    name: String,
    vram: u64,
    driver: String,
    virtual_display: bool,
    width: u64,
    height: u64,
    refresh: u64,
    driver_date: String,
    vendor: String,
) -> serde_json::Value {
    json!({
        "name": name,
        "vram": vram,
        "driver": driver,
        "virtual": virtual_display,
        "width": width,
        "height": height,
        "refresh": refresh,
        "driver_date": driver_date,
        "vendor": vendor,
    })
}

fn gpus_from_snapshot(
    rows: &[serde_json::Value],
    reg: &std::collections::HashMap<String, (u64, String, String)>,
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for g in rows {
        let name = json_text(g, "Name");
        if name.is_empty() {
            continue;
        }
        let extra = reg.get(&name.to_ascii_lowercase());
        let registry_mb = extra.map(|x| x.0).unwrap_or(0);
        let mut driver = json_text(g, "DriverVersion");
        if driver.is_empty() {
            driver = extra.map(|x| x.1.clone()).unwrap_or_default();
        }
        let vram = pick_vram_mb(jnum(g, "AdapterRAM", 0.0), registry_mb);
        out.push(gpu_value(
            name.clone(),
            vram,
            driver,
            is_virtual_gpu(&name),
            json_u64(g, "Width"),
            json_u64(g, "Height"),
            json_u64(g, "Refresh"),
            parse_wmi_date(&json_text(g, "DriverDate")),
            json_text(g, "Vendor"),
        ));
    }
    if out.is_empty() {
        for (_key, (mb, driver, name)) in reg {
            if *mb == 0 || is_virtual_gpu(name) {
                continue;
            }
            out.push(gpu_value(
                name.clone(),
                *mb,
                driver.clone(),
                false,
                0,
                0,
                0,
                String::new(),
                String::new(),
            ));
        }
    }
    let mut best: Vec<serde_json::Value> = Vec::new();
    for g in out {
        if let Some(ex) = best.iter_mut().find(|e| e["name"] == g["name"]) {
            if g["vram"].as_u64().unwrap_or(0) > ex["vram"].as_u64().unwrap_or(0) {
                *ex = g;
            }
        } else {
            best.push(g);
        }
    }
    best.sort_by_key(|g| g["virtual"].as_bool().unwrap_or(false));
    best
}

fn disks_from_snapshot(rows: &[serde_json::Value], phys: &[serde_json::Value]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|d| {
            let model = json_text(d, "Model");
            let size = json_u64(d, "Size");
            let matched = phys.iter().find(|p| {
                let friendly = json_text(p, "FriendlyName");
                !friendly.is_empty()
                    && (friendly.eq_ignore_ascii_case(model.trim())
                        || model.to_ascii_lowercase().contains(&friendly.to_ascii_lowercase()))
            });
            let bus = matched
                .map(|p| map_storage_enum("bus", &json_text(p, "BusType")))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| json_text(d, "InterfaceType"));
            let media = matched
                .map(|p| map_storage_enum("media", &json_text(p, "MediaType")))
                .unwrap_or_default();
            let health = matched
                .map(|p| map_storage_enum("health", &json_text(p, "HealthStatus")))
                .unwrap_or_default();
            let mut serial = json_text(d, "SerialNumber").trim().to_string();
            if serial.is_empty() {
                serial = matched
                    .map(|p| json_text(p, "SerialNumber").trim().to_string())
                    .unwrap_or_default();
            }
            let firmware = json_text(d, "FirmwareRevision").trim().to_string();
            json!({
                "model": if model.is_empty() { "未知磁盘".to_string() } else { model },
                "size": size,
                "bus": bus,
                "media": media,
                "health": health,
                "serial": serial,
                "firmware": firmware,
            })
        })
        .collect()
}

fn memory_from_snapshot(rows: &[serde_json::Value]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|m| {
            let manufacturer = json_text(m, "Manufacturer").trim().to_string();
            let slot = json_text(m, "DeviceLocator").trim().to_string();
            let kind = mem_type_name(json_u64(m, "SMBIOSMemoryType"));
            let part = json_text(m, "PartNumber").trim().to_string();
            json!({
                "manufacturer": manufacturer,
                "capacity_gb": (json_u64(m, "Capacity") as f64 / 1073741824.0 * 10.0).round() / 10.0,
                "speed": json_u64(m, "Speed"),
                "configured_mhz": json_u64(m, "ConfiguredClockSpeed"),
                "slot": slot,
                "bank": json_text(m, "BankLabel").trim(),
                "part_number": part,
                "form_factor": form_factor_name(json_u64(m, "FormFactor")),
                "data_width": json_u64(m, "DataWidth"),
                "type": kind,
            })
        })
        .filter(|m| m["capacity_gb"].as_f64().unwrap_or(0.0) > 0.0 || !json_text(m, "manufacturer").is_empty())
        .collect()
}

fn clean_id(s: String) -> String {
    if nonempty(&s) { s } else { String::new() }
}

fn cpu_arch_label(cpu: &serde_json::Value) -> String {
    match cpu.get("arch") {
        Some(serde_json::Value::Number(_)) | Some(serde_json::Value::String(_)) => {
            cpu_arch_name(json_u64(cpu, "arch")).to_string()
        }
        _ => String::new(),
    }
}

fn cpu_from_snapshot(
    cpu: &serde_json::Value,
    fallback_name: &str,
    fallback_cores: u64,
    fallback_threads: u64,
) -> serde_json::Value {
    let from_cim = json_text(cpu, "name");
    let name = if nonempty(&from_cim) {
        from_cim
    } else {
        fallback_name.trim().to_string()
    };
    let cores = json_u64(cpu, "cores");
    let threads = json_u64(cpu, "threads");
    json!({
        "name": name,
        "vendor": cpu_vendor_name(&json_text(cpu, "vendor")),
        "arch": cpu_arch_label(cpu),
        "socket": clean_id(json_text(cpu, "socket")),
        "cores": if cores > 0 { cores } else { fallback_cores },
        "threads": if threads > 0 { threads } else { fallback_threads },
        "mhz": json_u64(cpu, "mhz"),
        "current_mhz": json_u64(cpu, "current_mhz"),
        "l2_kb": json_u64(cpu, "l2_kb"),
        "l3_kb": json_u64(cpu, "l3_kb"),
        "virtualization": json_bool(cpu, "virtualization"),
    })
}

/// 硬件信息总览（CPU / 内存 / 主板 / BIOS / 显卡 / 磁盘 / 分区）
#[tauri::command]
pub async fn hw_info() -> serde_json::Value {
    use sysinfo::System;
    let mut sys = System::new_all();
    sys.refresh_cpu_usage();

    let cpus = sys.cpus();
    let sys_threads = cpus.len();
    let sys_brand = cpus
        .first()
        .map(|c| c.brand().split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let sys_cores = sys.physical_core_count().unwrap_or(sys_threads);
    let memory_total = sys.total_memory();

    let snap = cim_snapshot().unwrap_or(serde_json::Value::Null);
    let reg_gpu = registry_gpu_extra();
    let cpu_node = snap.get("cpu").cloned().unwrap_or(serde_json::Value::Null);
    let board_node = snap.get("board").cloned().unwrap_or(serde_json::Value::Null);
    let bios_node = snap.get("bios").cloned().unwrap_or(serde_json::Value::Null);
    let os_node = snap.get("os").cloned().unwrap_or(serde_json::Value::Null);

    let fallback_name = {
        let from_reg = cpu_name_registry();
        if nonempty(&from_reg) { from_reg } else { sys_brand }
    };
    let cpu = cpu_from_snapshot(&cpu_node, &fallback_name, sys_cores as u64, sys_threads as u64);

    let board_mfr = clean_id(json_text(&board_node, "manufacturer"));
    let board_product = clean_id(json_text(&board_node, "product"));
    let board_ver = clean_id(json_text(&board_node, "version"));
    let board_serial = clean_id(json_text(&board_node, "serial"));
    let board = [board_mfr.as_str(), board_product.as_str(), board_ver.as_str()]
        .into_iter()
        .filter(|s| nonempty(s))
        .collect::<Vec<_>>()
        .join(" ");
    let bios_vendor = clean_id(json_text(&bios_node, "vendor"));
    let bios_ver = clean_id(json_text(&bios_node, "version"));
    let bios_date = parse_wmi_date(&json_text(&bios_node, "date"));

    let gpus = gpus_from_snapshot(&as_array(snap.get("gpus").cloned()), &reg_gpu);
    let disks = disks_from_snapshot(
        &as_array(snap.get("disks").cloned()),
        &as_array(snap.get("phys").cloned()),
    );
    let memory_modules = memory_from_snapshot(&as_array(snap.get("mem").cloned()));

    json!({
        "cpu": cpu,
        "memory_total": memory_total,
        "memoryModules": memory_modules,
        "motherboard": {
            "manufacturer": board_mfr,
            "product": board_product,
            "version": board_ver,
            "serial": board_serial,
        },
        "board": board,
        "bios": { "vendor": bios_vendor, "version": bios_ver, "date": bios_date },
        "os": {
            "caption": json_text(&os_node, "caption").trim(),
            "version": json_text(&os_node, "version").trim(),
            "build": json_text(&os_node, "build").trim(),
            "arch": json_text(&os_node, "arch").trim(),
        },
        "gpus": gpus,
        "disks": disks,
        "volumes": volumes_info(),
    })
}

/// 轻量占用：CPU 需要两次采样才有值，页面轮询时复用同一份 System。
fn sample_usage() -> serde_json::Value {
    use sysinfo::System;
    static SYS: Mutex<Option<System>> = Mutex::new(None);
    let mut guard = SYS.lock().unwrap_or_else(|e| e.into_inner());
    let sys = guard.get_or_insert_with(System::new);
    sys.refresh_memory();
    // 空列表时先把 CPU 建出来；占用要靠下一次轮询的差值才准。
    if sys.cpus().is_empty() {
        sys.refresh_cpu_all();
    } else {
        sys.refresh_cpu_usage();
    }
    let pct = sys.global_cpu_usage();
    json!({
        "cpu_pct": (pct * 10.0).round() / 10.0,
        "mem_used": sys.used_memory(),
        "mem_total": sys.total_memory(),
    })
}

/// 实时 CPU / 内存占用。不走 WMI，供界面两三秒轮询。
#[tauri::command]
pub async fn hw_usage() -> serde_json::Value {
    tauri::async_runtime::spawn_blocking(sample_usage)
        .await
        .unwrap_or_else(|_| json!({ "cpu_pct": 0.0, "mem_used": 0, "mem_total": 0 }))
}

// ---------- CPU 烤机 ----------

fn stop_burn() {
    BURN_RUN.store(false, Ordering::Relaxed);
    if let Ok(mut v) = BURN_WORKERS.lock() {
        for h in v.drain(..) {
            let _ = h.join();
        }
    }
    if let Ok(mut m) = BURN_MONITOR.lock() {
        if let Some(h) = m.take() {
            let _ = h.join();
        }
    }
}

/// 启动 CPU 烤机（多线程忙运算，seconds 秒后自动停止；每秒发 cpu-load {pct} 事件）
#[tauri::command]
pub fn cpu_burn_start(app: tauri::AppHandle, seconds: u64) {
    // 返回前先停旧任务
    stop_burn();
    BURN_RUN.store(true, Ordering::Relaxed);

    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    if let Ok(mut v) = BURN_WORKERS.lock() {
        for _ in 0..n {
            v.push(std::thread::spawn(|| {
                let mut x = 1.5f64;
                while BURN_RUN.load(Ordering::Relaxed) {
                    for _ in 0..50_000 {
                        x = x * 1.0000001 + 0.0000001;
                        if x > 1e9 {
                            x = 1.0;
                        }
                    }
                    std::hint::black_box(x);
                }
            }));
        }
    }

    // 监控线程：采样真实 CPU 负载率 + 温度，超温自动终止
    let monitor_app = app;
    let max_temp = max_temp_setting();
    if let Ok(mut m) = BURN_MONITOR.lock() {
        m.replace(std::thread::spawn(move || {
            use sysinfo::System;
            let mut sys = System::new_all();
            let start = Instant::now();
            let mut peak = 0.0f64;
            let mut over_temp = false;
            while BURN_RUN.load(Ordering::Relaxed) && start.elapsed().as_secs() < seconds {
                std::thread::sleep(Duration::from_millis(1000));
                sys.refresh_cpu_usage();
                let pct = sys.global_cpu_usage();

                // 每 4 秒读一次温度：WMI 查询较重，太频繁会拖慢压测本身
                let mut temp: Option<f64> = None;
                if start.elapsed().as_secs() % 4 == 0 {
                    temp = read_cpu_temp_sync();
                    if let Some(t) = temp {
                        if t > peak {
                            peak = t;
                        }
                        // 超过硬阈值立即停：文档要求「超温自动终止，防止硬件过热」
                        if max_temp > 0.0 && t >= max_temp {
                            over_temp = true;
                            BURN_RUN.store(false, Ordering::Relaxed);
                        }
                    }
                }

                let _ = monitor_app.emit(
                    "cpu-load",
                    json!({ "pct": pct, "temp": temp, "peak": peak }),
                );
            }
            BURN_RUN.store(false, Ordering::Relaxed);
            // 通知前端是「到时结束」还是「超温保护」
            let _ = monitor_app.emit(
                "cpu-burn-end",
                json!({ "over_temp": over_temp, "peak": round1(peak), "limit": max_temp }),
            );
        }));
    }
}

/// 读设置里的温度上限；未配置或非法时回退 95℃（多数桌面 CPU 的 Tjmax 附近）
fn max_temp_setting() -> f64 {
    let v = crate::modules::common::read_setting_raw("burn_max_temp_c");
    let parsed = v
        .as_ref()
        .and_then(|x| match x {
            serde_json::Value::Number(n) => n.as_f64(),
            serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        })
        .unwrap_or(0.0);
    resolve_max_temp(parsed)
}

/// 温度上限合法性判定（抽成纯函数便于单测）。
/// 允许 50-110℃，范围外一律回退 95℃；0 或负数视为未配置。
fn resolve_max_temp(raw: f64) -> f64 {
    if (50.0..=110.0).contains(&raw) {
        raw
    } else {
        95.0
    }
}

/// 停止 CPU 烤机
#[tauri::command]
pub fn cpu_burn_stop() {
    stop_burn();
}

// ---------- 磁盘测速 ----------

/// 磁盘测速：写 512MB 临时文件测写速，读回测读速，测毕删除
#[tauri::command]
pub async fn disk_speed(path: String) -> Result<DiskSpeed, String> {
    use std::io::{Read, Write};

    // 兼容多种输入：目录 / 盘符 "C" / "C:" / "C:\"；无效时回退系统临时目录
    let mut dir = PathBuf::from(&path);
    if !dir.is_dir() {
        let p = path.trim();
        dir = if p.len() >= 2 && p.as_bytes()[1] == b':' {
            PathBuf::from(format!("{}\\", &p[..2]))
        } else {
            std::env::temp_dir()
        };
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法使用目录 {}: {}", dir.display(), e))?;
    let file_path = dir.join("jwspeed.tmp");

    let total: u64 = 512 * 1024 * 1024;
    // 4MB 伪随机块（避免被文件系统/压缩优化）
    let mut data = vec![0u8; 4 * 1024 * 1024];
    let mut seed = 0x12345678u32;
    for b in data.iter_mut() {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *b = (seed >> 24) as u8;
    }

    // 写测速
    let start = Instant::now();
    {
        let mut f = std::fs::File::create(&file_path).map_err(|e| format!("创建临时文件失败: {}", e))?;
        let mut written = 0u64;
        while written < total {
            let n = ((total - written) as usize).min(data.len());
            f.write_all(&data[..n]).map_err(|e| format!("写入失败: {}", e))?;
            written += n as u64;
        }
        f.sync_all().map_err(|e| format!("刷盘失败: {}", e))?;
    }
    let write_mbps = total as f64 / 1048576.0 / start.elapsed().as_secs_f64();

    // 读测速
    let start = Instant::now();
    {
        let mut f = std::fs::File::open(&file_path).map_err(|e| format!("读取失败: {}", e))?;
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        loop {
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => return Err(format!("读取失败: {}", e)),
            }
        }
    }
    let read_mbps = total as f64 / 1048576.0 / start.elapsed().as_secs_f64();

    let _ = std::fs::remove_file(&file_path);
    Ok(DiskSpeed { read_mbps, write_mbps })
}

// ---------- GPU 烤机（wgpu compute） ----------

/// 矩阵边长（256x256 f32，单次 dispatch ≈ 33.5 MFLOP）
const MAT_N: u32 = 256;
/// 每次 submit 重复 dispatch 的批次数（避免 CPU 提交开销成为瓶颈）
const GPU_BATCH: u32 = 64;

const GPU_SHADER: &str = r#"
const N: u32 = 256u;
@group(0) @binding(0) var<storage, read> a: array<f32>;
@group(0) @binding(1) var<storage, read> b: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let row = gid.y;
    let col = gid.x;
    if (row >= N || col >= N) {
        return;
    }
    var sum = 0.0;
    for (var k: u32 = 0u; k < N; k = k + 1u) {
        sum = sum + a[row * N + k] * b[k * N + col];
    }
    out[row * N + col] = sum;
}
"#;

/// 极简 block_on（noop waker 忙等，仅用于初始化阶段的同步等待）
fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }
    let waker = Waker::from(Arc::new(NoopWake));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::sleep(Duration::from_millis(2)),
        }
    }
}

struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
}

impl GpuContext {
    fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| "未找到可用的 GPU 适配器".to_string())?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("jw-gpu-burn"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
        }, None))
        .map_err(|e| e.to_string())?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("jw-gpu-burn.wgsl"),
            source: wgpu::ShaderSource::Wgsl(GPU_SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("jw-gpu-burn-pipeline"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let elem = MAT_N as u64 * MAT_N as u64;
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let mk = |usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: elem * 4,
                usage,
                mapped_at_creation: false,
            })
        };
        let (buf_a, buf_b, buf_out) = (mk(usage), mk(usage), mk(wgpu::BufferUsages::STORAGE));

        // 填充输入数据（LCG 伪随机，避免全零被优化）
        let mut seed = 0x12345678u32;
        let mut data = vec![0u8; (elem * 4) as usize];
        for chunk in data.chunks_exact_mut(4) {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            chunk.copy_from_slice(&seed.to_ne_bytes());
        }
        queue.write_buffer(&buf_a, 0, &data);
        queue.write_buffer(&buf_b, 0, &data);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buf_a.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: buf_b.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: buf_out.as_entire_binding() },
            ],
            label: None,
        });

        Ok(GpuContext { device, queue, pipeline, bind_group })
    }
}

static GPU_RUN: AtomicBool = AtomicBool::new(false);
static GPU_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

fn stop_gpu_burn() {
    GPU_RUN.store(false, Ordering::Relaxed);
    if let Ok(mut t) = GPU_THREAD.lock() {
        if let Some(h) = t.take() {
            let _ = h.join();
        }
    }
}

/// 启动 GPU 烤机：compute 大矩阵乘法循环，seconds 秒后自动停止；
/// 每 500ms 发 gpu-load {pct, fps}（pct 恒 100，fps 为每秒迭代次数）
#[tauri::command]
pub fn gpu_burn_start(app: tauri::AppHandle, seconds: u64) -> Result<(), String> {
    stop_gpu_burn();
    GPU_RUN.store(true, Ordering::Relaxed);

    // 同步初始化，失败则上报并返回 Err
    let ctx = match GpuContext::new() {
        Ok(c) => c,
        Err(e) => {
            GPU_RUN.store(false, Ordering::Relaxed);
            let _ = app.emit("gpu-load", json!({ "error": "不支持" }));
            return Err(format!("当前环境不支持 wgpu: {}", e));
        }
    };

    if let Ok(mut t) = GPU_THREAD.lock() {
        t.replace(std::thread::spawn(move || {
            let start = Instant::now();
            let mut last_emit = Instant::now();
            let mut window_iters: u64 = 0;
            let mut window_start = Instant::now();
            while GPU_RUN.load(Ordering::Relaxed) && start.elapsed().as_secs() < seconds {
                let mut encoder = ctx
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: None,
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&ctx.pipeline);
                    pass.set_bind_group(0, &ctx.bind_group, &[]);
                    for _ in 0..GPU_BATCH {
                        pass.dispatch_workgroups(MAT_N / 16, MAT_N / 16, 1);
                    }
                }
                ctx.queue.submit(Some(encoder.finish()));
                let _ = ctx.device.poll(wgpu::Maintain::Wait);
                window_iters += GPU_BATCH as u64;

                if last_emit.elapsed() >= Duration::from_millis(500) {
                    let secs = window_start.elapsed().as_secs_f64().max(0.001);
                    let fps = (window_iters as f64 / secs).round() as u64;
                    window_iters = 0;
                    window_start = Instant::now();
                    last_emit = Instant::now();
                    let _ = app.emit("gpu-load", json!({ "pct": 100, "fps": fps }));
                }
            }
            GPU_RUN.store(false, Ordering::Relaxed);
        }));
    }
    Ok(())
}

/// 停止 GPU 烤机
#[tauri::command]
pub fn gpu_burn_stop() -> Result<(), String> {
    stop_gpu_burn();
    Ok(())
}

// ---------- 磁盘 SMART 健康 ----------

/// OperationalStatus 可能是字符串或数组，统一为字符串
fn jany(v: &serde_json::Value, key: &str, default: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        _ => default.to_string(),
    }
}

/// 取可选数值字段（WMI 侧缺失/为 null 时返回 None）
fn jopt(v: Option<&serde_json::Value>, key: &str) -> Option<f64> {
    v.and_then(|x| x.get(key)).and_then(|x| x.as_f64())
}

/// 磁盘 SMART 健康：Get-PhysicalDisk + Get-StorageReliabilityCounter 按 DeviceId 对齐；
/// 部分盘无可靠性数据（需管理员/不支持）时相应字段为 null
#[tauri::command]
pub async fn disk_smart() -> Result<Vec<serde_json::Value>, String> {
    let disks = as_array(ps_json(
        r#"@(Get-PhysicalDisk | Select-Object DeviceId,FriendlyName,SerialNumber,BusType,MediaType,HealthStatus,OperationalStatus) | ConvertTo-Json -Compress"#,
    ));
    if disks.is_empty() {
        return Err("无法读取磁盘信息（需要管理员权限或硬盘不支持）".into());
    }
    let rel = as_array(ps_json(
        r#"@(Get-PhysicalDisk | Get-StorageReliabilityCounter | Select-Object DeviceId,Temperature,PowerOnHours,Wear) | ConvertTo-Json -Compress"#,
    ));

    let out: Vec<serde_json::Value> = disks
        .iter()
        .map(|d| {
            let dev = jstr(d, "DeviceId", "");
            let r = rel.iter().find(|x| jstr(x, "DeviceId", "") == dev);
            json!({
                "model": jstr(d, "FriendlyName", "未知"),
                "serial": jstr(d, "SerialNumber", ""),
                "bus_type": map_storage_enum("bus", &json_text(d, "BusType")),
                "media": map_storage_enum("media", &json_text(d, "MediaType")),
                "health": map_storage_enum("health", &json_text(d, "HealthStatus")),
                "status": jany(d, "OperationalStatus", ""),
                "temperature_c": jopt(r, "Temperature"),
                "power_on_hours": jopt(r, "PowerOnHours").map(|v| v as u64),
                "wear_pct": jopt(r, "Wear"),
            })
        })
        .collect();
    Ok(out)
}

// ---------- 温度 ----------

/// 四舍五入到 0.1℃
fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// CPU 温度同步实现：sysinfo 温度组件（名含 CPU/Core 取均值）→ WMI 热区回退 → None
/// 压测监控线程需要同步调用以做超温保护，故单独抽出。
fn read_cpu_temp_sync() -> Option<f64> {
    // sysinfo（Windows 上支持有限，取不到则走 WMI）
    let comps = sysinfo::Components::new_with_refreshed_list();
    let vals: Vec<f64> = comps
        .list()
        .iter()
        .filter(|c| {
            let l = c.label().to_lowercase();
            l.contains("cpu") || l.contains("core")
        })
        .filter_map(|c| c.temperature())
        .map(|t| t as f64)
        .filter(|t| *t > 0.0 && *t < 150.0)
        .collect();
    if !vals.is_empty() {
        let n = vals.len() as f64;
        return Some(round1(vals.iter().sum::<f64>() / n));
    }

    // WMI 热区回退：CurrentTemperature 为十分之一开尔文
    let zones = as_array(ps_json(
        r#"@(Get-CimInstance MSAcpi_ThermalZoneTemperature -Namespace root/wmi | Select-Object CurrentTemperature) | ConvertTo-Json -Compress"#,
    ));
    let vals: Vec<f64> = zones
        .iter()
        .filter_map(|z| z.get("CurrentTemperature")?.as_f64())
        .map(|k| k / 10.0 - 273.15)
        .filter(|t| *t > 0.0 && *t < 150.0)
        .collect();
    if vals.is_empty() {
        return None;
    }
    let n = vals.len() as f64;
    Some(round1(vals.iter().sum::<f64>() / n))
}

/// CPU 温度
#[tauri::command]
pub async fn cpu_temp() -> Option<f64> {
    read_cpu_temp_sync()
}

/// GPU 温度：nvidia-smi（PATH → System32 → NVSMI 安装目录），不可用返回 null
#[tauri::command]
pub async fn gpu_temp() -> Option<f64> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    for prog in [
        "nvidia-smi",
        r"C:\Windows\System32\nvidia-smi.exe",
        r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
    ] {
        let out = std::process::Command::new(prog)
            .args(["--query-gpu=temperature.gpu", "--format=csv,noheader,nounits"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Ok(out) = out {
            if out.status.success() {
                if let Ok(s) = String::from_utf8(out.stdout) {
                    if let Some(t) = s.lines().next().and_then(|l| l.trim().parse::<f64>().ok()) {
                        if t > 0.0 && t < 150.0 {
                            return Some(round1(t));
                        }
                    }
                }
            }
        }
    }
    None
}

// ---------- 内存测试 ----------

static MEM_TEST_RUN: AtomicBool = AtomicBool::new(false);

/// 启动内存测试：分配大块内存写入 0xAA/0x55/随机模式并读回校验；
/// 逐块发 mem-test-progress {pct, errors}，完成发 mem-test-done {errors, blocks_total, seconds}
#[tauri::command]
pub fn mem_test_start(app: tauri::AppHandle) -> Result<(), String> {
    if MEM_TEST_RUN.swap(true, Ordering::SeqCst) {
        return Err("内存测试正在进行中".into());
    }
    std::thread::spawn(move || {
        run_mem_test(app);
        MEM_TEST_RUN.store(false, Ordering::SeqCst);
    });
    Ok(())
}

fn run_mem_test(app: tauri::AppHandle) {
    use sysinfo::System;

    // 分配 min(可用内存 60%, 2GB)，按 64MB 分块
    let avail = System::new_all().available_memory();
    let total = ((avail as f64 * 0.6) as u64).min(2 * 1024 * 1024 * 1024) as usize;
    let block = total.min(64 * 1024 * 1024).max(1024 * 1024);
    let blocks = (total / block).max(1);

    let start = Instant::now();
    let mut errors: u64 = 0;
    let mut seed = 0xDEADBEEFu32;

    for i in 0..blocks {
        let mut buf: Vec<u8> = Vec::new();
        if buf.try_reserve_exact(block).is_err() {
            break; // 分配失败，结束并上报已有结果
        }
        buf.resize(block, 0);

        // 固定模式 0xAA / 0x55
        for pattern in [0xAAu8, 0x55u8] {
            buf.fill(pattern);
            errors += buf.iter().filter(|&&b| b != pattern).count() as u64;
        }

        // 随机模式写入 → 用同一序列回读校验
        let mut wseed = seed;
        for b in buf.iter_mut() {
            wseed = wseed.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = (wseed >> 24) as u8;
        }
        let mut rseed = seed;
        for &b in buf.iter() {
            rseed = rseed.wrapping_mul(1664525).wrapping_add(1013904223);
            if (rseed >> 24) as u8 != b {
                errors += 1;
            }
        }
        seed = seed.wrapping_add(0x9E3779B9);

        let _ = app.emit(
            "mem-test-progress",
            json!({ "pct": ((i + 1) * 100 / blocks), "errors": errors }),
        );
    }

    let _ = app.emit(
        "mem-test-done",
        json!({
            "errors": errors,
            "blocks_total": blocks,
            "seconds": start.elapsed().as_secs(),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::{
        cpu_arch_name, cpu_from_snapshot, cpu_vendor_name, disks_from_snapshot, form_factor_name,
        gpus_from_snapshot, is_virtual_gpu, map_storage_enum, mem_type_name, memory_from_snapshot,
        parse_wmi_date, pick_vram_mb, resolve_max_temp, vram_mb, DiskSpeed,
    };
    use serde_json::json;

    #[test]
    fn vram_bytes_to_mb() {
        assert_eq!(vram_mb(8.0 * 1024.0 * 1024.0 * 1024.0), 8192);
        assert_eq!(vram_mb(512.0 * 1024.0 * 1024.0), 512);
    }

    #[test]
    fn vram_rejects_overflow_and_negative() {
        // WMI uint32 溢出时常见 -1 或超大值
        assert_eq!(vram_mb(-1.0), 0);
        assert_eq!(vram_mb(0.0), 0);
        assert_eq!(vram_mb(128.0 * 1024.0 * 1024.0 * 1024.0), 0);
    }

    #[test]
    fn disk_speed_json_keeps_snake_case() {
        // 前端读 read_mbps / write_mbps，不能被改成 readMbps
        let v = serde_json::to_value(DiskSpeed { read_mbps: 1.5, write_mbps: 2.5 }).unwrap();
        assert_eq!(v["read_mbps"], 1.5);
        assert_eq!(v["write_mbps"], 2.5);
    }

    #[test]
    fn vram_prefers_registry_when_wmi_saturates() {
        // RTX 3060 一类显卡：WMI AdapterRAM 停在 4GB 以下，注册表 qwMemorySize 才是 12GB
        assert_eq!(pick_vram_mb(4_293_918_720.0, 12288), 12288);
        assert_eq!(pick_vram_mb(512.0 * 1024.0 * 1024.0, 0), 512);
    }

    #[test]
    fn virtual_display_is_not_a_real_gpu() {
        assert!(is_virtual_gpu("OrayIddDriver Device"));
        assert!(is_virtual_gpu("AskLink Display Adapter (SDR)"));
        assert!(is_virtual_gpu("GameViewer Virtual Display Adapter"));
        assert!(!is_virtual_gpu("NVIDIA GeForce RTX 3060"));
    }

    #[test]
    fn memory_and_disk_enums_are_readable() {
        assert_eq!(mem_type_name(26), "DDR4");
        assert_eq!(mem_type_name(34), "DDR5");
        assert_eq!(map_storage_enum("bus", "17"), "NVMe");
        assert_eq!(map_storage_enum("media", "4"), "SSD");
        assert_eq!(map_storage_enum("health", "0"), "Healthy");
        assert_eq!(map_storage_enum("bus", "NVMe"), "NVMe");
    }

    #[test]
    fn wmi_dates_normalize() {
        assert_eq!(parse_wmi_date("2024-03-06"), "2024-03-06");
        assert_eq!(parse_wmi_date("/Date(1709683200000)/"), "2024-03-06");
        assert_eq!(parse_wmi_date("20240306000000.000000+000"), "2024-03-06");
    }

    #[test]
    fn cpu_snapshot_keeps_cache_clock_and_virtualization() {
        assert_eq!(cpu_vendor_name("GenuineIntel"), "Intel");
        assert_eq!(cpu_vendor_name("AuthenticAMD"), "AMD");
        assert_eq!(form_factor_name(8), "DIMM");
        assert_eq!(form_factor_name(12), "SO-DIMM");
        assert_eq!(cpu_arch_name(9), "x64");
        assert_eq!(cpu_arch_name(12), "ARM64");

        let raw = json!({
            "name": "Intel Core i5-14600KF",
            "cores": 14,
            "threads": 20,
            "mhz": 3500,
            "current_mhz": 4700,
            "l2_kb": 20480,
            "l3_kb": 24576,
            "socket": "U3E1",
            "vendor": "GenuineIntel",
            "arch": 9,
            "virtualization": true
        });
        let v = cpu_from_snapshot(&raw, "fallback", 1, 1);
        assert_eq!(v["name"], "Intel Core i5-14600KF");
        assert_eq!(v["vendor"], "Intel");
        assert_eq!(v["arch"], "x64");
        assert_eq!(v["l3_kb"], 24576);
        assert_eq!(v["current_mhz"], 4700);
        assert_eq!(v["virtualization"], true);

        let missing = cpu_from_snapshot(&json!({}), "fallback", 4, 8);
        assert_eq!(missing["name"], "fallback");
        assert_eq!(missing["arch"], "");
        assert!(missing["virtualization"].is_null());
    }

    #[test]
    fn memory_module_keeps_part_and_form() {
        let rows = vec![json!({
            "Manufacturer": "Samsung",
            "DeviceLocator": "Controller0-DIMM1",
            "BankLabel": "BANK 0",
            "PartNumber": "M378A2K43EB1-CWE  ",
            "Speed": 3200,
            "ConfiguredClockSpeed": 3200,
            "Capacity": 17179869184u64,
            "SMBIOSMemoryType": 26,
            "FormFactor": 8,
            "DataWidth": 64
        })];
        let v = memory_from_snapshot(&rows);
        assert_eq!(v[0]["type"], "DDR4");
        assert_eq!(v[0]["form_factor"], "DIMM");
        assert_eq!(v[0]["part_number"], "M378A2K43EB1-CWE");
        assert_eq!(v[0]["configured_mhz"], 3200);
        assert_eq!(v[0]["capacity_gb"], 16.0);
        assert_eq!(v[0]["data_width"], 64);
    }

    #[test]
    fn gpu_keeps_resolution_and_driver_date() {
        let rows = vec![json!({
            "Name": "NVIDIA GeForce RTX 3060",
            "AdapterRAM": 4293918720.0,
            "DriverVersion": "32.0.15.6094",
            "DriverDate": "2024-11-01",
            "Width": 1920,
            "Height": 1080,
            "Refresh": 144,
            "Vendor": "NVIDIA"
        })];
        let reg = std::collections::HashMap::new();
        let v = gpus_from_snapshot(&rows, &reg);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0]["width"], 1920);
        assert_eq!(v[0]["height"], 1080);
        assert_eq!(v[0]["refresh"], 144);
        assert_eq!(v[0]["driver_date"], "2024-11-01");
        assert_eq!(v[0]["virtual"], false);
        assert_eq!(v[0]["vram"], 4095);
    }

    #[test]
    fn disk_keeps_serial_and_firmware() {
        let rows = vec![json!({
            "Model": "WD PC SN810 SDCPNRY-512G",
            "Size": 512110190592u64,
            "InterfaceType": "SCSI",
            "SerialNumber": "  SN123  ",
            "FirmwareRevision": "53006100"
        })];
        let phys = vec![json!({
            "FriendlyName": "WD PC SN810 SDCPNRY-512G",
            "MediaType": 4,
            "BusType": 17,
            "HealthStatus": 0,
            "Size": 512110190592u64
        })];
        let v = disks_from_snapshot(&rows, &phys);
        assert_eq!(v[0]["bus"], "NVMe");
        assert_eq!(v[0]["media"], "SSD");
        assert_eq!(v[0]["health"], "Healthy");
        assert_eq!(v[0]["serial"], "SN123");
        assert_eq!(v[0]["firmware"], "53006100");
    }

    #[test]
    fn max_temp_setting_falls_back_to_safe_default() {
        // 合法范围保留用户设置
        assert_eq!(resolve_max_temp(85.0), 85.0);
        assert_eq!(resolve_max_temp(50.0), 50.0);
        assert_eq!(resolve_max_temp(110.0), 110.0);
        // 未配置 / 过低 / 过高一律回退 95℃，避免「关掉保护」
        assert_eq!(resolve_max_temp(0.0), 95.0);
        assert_eq!(resolve_max_temp(-10.0), 95.0);
        assert_eq!(resolve_max_temp(30.0), 95.0);
        assert_eq!(resolve_max_temp(150.0), 95.0);
    }
}
