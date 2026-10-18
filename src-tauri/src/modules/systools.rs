//! 系统工具：启动项 / 服务 / hosts / 运行库与 UWP（P1 桩）

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use tauri::Emitter;
use winreg::enums::*;
use winreg::HKEY;
use winreg::RegKey;

// ---------- 启动项 ----------

#[derive(Serialize)]
pub struct StartupItem {
    pub name: String,
    pub command: String,
    pub location: String,
}

const RUN_PATH_HKCU: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_PATH_HKLM: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
const RUN_PATH_WOW: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";

/// list_startup 时的快照缓存，toggle_startup(enable) 时用于还原命令
static STARTUP_CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn collect_run(root: HKEY, path: &str, label: &str, out: &mut Vec<StartupItem>, cache: &mut HashMap<String, String>) {
    if let Ok(k) = RegKey::predef(root).open_subkey(path) {
        for (name, value) in k.enum_values().flatten() {
            let cmd = value.to_string();
            cache.insert(name.clone(), cmd.clone());
            out.push(StartupItem {
                name,
                command: cmd,
                location: format!("{}\\{}", label, path),
            });
        }
    }
}

/// 列出启动项（HKCU/HKLM Run + 启动文件夹 .lnk）
#[tauri::command]
pub async fn list_startup() -> Vec<StartupItem> {
    let mut out = Vec::new();
    let mut cache = HashMap::new();

    collect_run(HKEY_CURRENT_USER, RUN_PATH_HKCU, "HKCU", &mut out, &mut cache);
    collect_run(HKEY_LOCAL_MACHINE, RUN_PATH_HKLM, "HKLM", &mut out, &mut cache);
    collect_run(HKEY_LOCAL_MACHINE, RUN_PATH_WOW, "HKLM", &mut out, &mut cache);

    // 启动文件夹快捷方式
    let folders: Vec<std::path::PathBuf> = [
        std::env::var("APPDATA").ok(),
        std::env::var("ProgramData").ok(),
    ]
    .into_iter()
    .flatten()
    .map(|p| Path::new(&p).join(r"Microsoft\Windows\Start Menu\Programs\Startup"))
    .collect();
    for folder in folders {
        if let Ok(entries) = std::fs::read_dir(&folder) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
                    let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    cache.insert(name.clone(), p.to_string_lossy().into_owned());
                    out.push(StartupItem {
                        name,
                        command: p.to_string_lossy().into_owned(),
                        location: folder.to_string_lossy().into_owned(),
                    });
                }
            }
        }
    }

    if let Ok(mut c) = STARTUP_CACHE.lock() {
        *c = Some(cache);
    }
    out
}

/// 启用/禁用启动项（按 name 匹配 Run 值；enable 时按 list_startup 缓存的命令写回 HKCU）
#[tauri::command]
pub fn toggle_startup(name: String, enable: bool) -> Result<(), String> {
    let dirs = crate::dirs::load_dirs();
    let action = if enable { "启用" } else { "禁用" };
    let r = toggle_startup_inner(name.clone(), enable);
    let outcome = match &r {
        Ok(_) => "成功".to_string(),
        Err(e) => format!("失败: {}", e),
    };
    crate::logs::op_log(&dirs, "systools", "toggle_startup", &format!("启动项 {} {}: {}", name, action, outcome));
    r
}

fn toggle_startup_inner(name: String, enable: bool) -> Result<(), String> {
    if enable {
        let cmd = STARTUP_CACHE
            .lock()
            .ok()
            .and_then(|c| c.as_ref().and_then(|m| m.get(&name).cloned()))
            .ok_or_else(|| format!("未知启动项: {}", name))?;
        if cmd.to_lowercase().ends_with(".lnk") {
            return Err("启动文件夹快捷方式暂不支持启用恢复".into());
        }
        let run = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_PATH_HKCU, KEY_SET_VALUE)
            .map_err(|e| e.to_string())?;
        run.set_value(&name, &cmd).map_err(|e| e.to_string())
    } else {
        let mut last_err = None;
        for (root, path) in [
            (HKEY_CURRENT_USER, RUN_PATH_HKCU),
            (HKEY_LOCAL_MACHINE, RUN_PATH_HKLM),
            (HKEY_LOCAL_MACHINE, RUN_PATH_WOW),
        ] {
            if let Ok(run) = RegKey::predef(root).open_subkey_with_flags(path, KEY_SET_VALUE) {
                match run.delete_value(&name) {
                    Ok(_) => return Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => last_err = Some(e.to_string()),
                }
            }
        }
        match last_err {
            Some(e) => Err(e),
            None => Err(format!("未找到启动项: {}", name)),
        }
    }
}

// ---------- Windows 服务 ----------

#[derive(Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub display: String,
    pub status: String,
    pub start_type: String,
}

/// 解析 Get-Service 的 ConvertTo-Json 输出为服务列表：
/// - 单结果时 ConvertTo-Json 输出对象（非数组）→ 包装为单元素数组
/// - 空数组时 PowerShell 输出空串 → 视为空列表
/// - 缺字段的条目被丢弃
fn parse_services_output(text: &str) -> Result<Vec<ServiceInfo>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let arr = match v {
        serde_json::Value::Array(a) => a,
        other => vec![other],
    };
    Ok(arr.into_iter().filter_map(|x| serde_json::from_value(x).ok()).collect())
}

/// 列出全部 Windows 服务（PowerShell Get-Service）
#[tauri::command]
pub async fn list_services() -> Result<Vec<ServiceInfo>, String> {
    let script = r#"[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; $r = Get-Service | ForEach-Object { @{ name = $_.Name; display = $_.DisplayName; status = [string]$_.Status; start_type = [string]$_.StartType } }; @($r) | ConvertTo-Json -Compress"#;
    let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_services_output(&text)
}

/// 服务操作：start / stop / auto / manual / disabled（sc 命令，需管理员）
#[tauri::command]
pub fn set_service(name: String, action: String) -> Result<(), String> {
    let dirs = crate::dirs::load_dirs();
    let args: Vec<&str> = match action.as_str() {
        "start" => vec!["start", &name],
        "stop" => vec!["stop", &name],
        "auto" => vec!["config", &name, "start=", "auto"],
        "manual" => vec!["config", &name, "start=", "demand"],
        "disabled" | "disable" => vec!["config", &name, "start=", "disabled"],
        _ => return Err(format!("不支持的服务操作: {}", action)),
    };
    let mut cmd = std::process::Command::new("sc");
    cmd.args(&args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let r = if out.status.success() {
        Ok(())
    } else {
        Err(format!("sc {}: {}", action, String::from_utf8_lossy(&out.stdout).trim()))
    };
    let outcome = match &r {
        Ok(_) => "成功".to_string(),
        Err(e) => format!("失败: {}", e),
    };
    crate::logs::op_log(&dirs, "systools", "set_service", &format!("服务 {} 执行 {}: {}", name, action, outcome));
    r
}

// ---------- hosts ----------

const HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";

/// 读取 hosts
#[tauri::command]
pub async fn read_hosts() -> Result<String, String> {
    let bytes = std::fs::read(HOSTS_PATH).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 写入 hosts（需管理员权限）
#[tauri::command]
pub fn write_hosts(content: String) -> Result<(), String> {
    let dirs = crate::dirs::load_dirs();
    let r = std::fs::write(HOSTS_PATH, &content).map_err(|e| format!("写入失败（需要管理员权限）: {}", e));
    let outcome = match &r {
        Ok(_) => "成功".to_string(),
        Err(e) => format!("失败: {}", e),
    };
    crate::logs::op_log(&dirs, "systools", "write_hosts", &format!("写入 hosts（{} 字节）: {}", content.len(), outcome));
    r
}

// ---------- 运行库检测与安装 ----------

static RUNTIME_INSTALLING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// VC++ 2015-2022 运行库版本（VisualStudio\14.0\VC\Runtimes\<arch> 的 Version 值）
fn vc_runtime_version(arch: &str) -> Option<String> {
    let paths = [
        format!(r"SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\{}", arch),
        format!(r"SOFTWARE\WOW6432Node\Microsoft\VisualStudio\14.0\VC\Runtimes\{}", arch),
    ];
    for p in paths {
        if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(&p) {
            if let Ok(v) = k.get_value::<String, _>("Version") {
                if !v.trim().is_empty() {
                    return Some(v.trim_start_matches('v').to_string());
                }
            }
        }
    }
    None
}

/// 旧版 VC++（2008/2010/2012/2013）检测：Uninstall 键 DisplayName 过滤，返回 (arch, version) 列表
fn detect_old_vc(year: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let keys = [
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ];
    for path in keys {
        if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(path) {
            for sub in k.enum_keys().flatten() {
                if let Ok(sk) = k.open_subkey(&sub) {
                    let name: String = sk.get_value("DisplayName").unwrap_or_default();
                    if !name.contains(&format!("Visual C++ {} Redistributable", year)) {
                        continue;
                    }
                    let arch = if name.contains("x64") {
                        "x64"
                    } else if path.contains("WOW6432Node") {
                        "x86"
                    } else {
                        "x86"
                    };
                    let ver: String = sk.get_value("DisplayVersion").unwrap_or_default();
                    let ver = if ver.trim().is_empty() {
                        name.rsplit('-').next().unwrap_or("").trim().to_string()
                    } else {
                        ver
                    };
                    if !out.iter().any(|(a, _): &(String, String)| a == arch) {
                        out.push((arch.to_string(), ver));
                    }
                }
            }
        }
    }
    out
}

/// .NET 检测：dotnet\Setup\InstalledVersions（.NET Core 及以上）+ NDP\v4\Full（.NET Framework 4.x）
fn detect_dotnet() -> (bool, String) {
    let mut best = String::new();
    // dotnet\Setup\InstalledVersions\x64|x86 下的版本型值
    for arch in ["x64", "x86"] {
        let path = format!(r"SOFTWARE\dotnet\Setup\InstalledVersions\{}", arch);
        if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(&path) {
            for (name, _) in k.enum_values().flatten() {
                if name.split('.').count() >= 2 && name.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                    && name > best
                {
                    best = name;
                }
            }
            for sub in k.enum_keys().flatten() {
                if let Ok(sk) = k.open_subkey(&sub) {
                    for (name, _) in sk.enum_values().flatten() {
                        if name.split('.').count() >= 2 && name.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                            && name > best
                        {
                            best = name;
                        }
                    }
                }
            }
        }
    }
    if !best.is_empty() {
        return (true, best);
    }
    // .NET Framework 4.x：NDP\v4\Full 的 Release DWORD
    if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Microsoft\NET Framework Setup\NDP\v4\Full") {
        if let Ok(release) = k.get_value::<u32, _>("Release") {
            let ver = if release >= 533320 {
                "4.8.1"
            } else if release >= 528040 {
                "4.8"
            } else if release >= 461808 {
                "4.7.2"
            } else {
                "4.7"
            };
            return (true, ver.to_string());
        }
    }
    (false, String::new())
}

/// 列出常见运行库及安装状态 [{id, name, installed, version, download_url}]
#[tauri::command]
pub async fn list_runtime_libs() -> Vec<serde_json::Value> {
    let mut out = Vec::new();

    // VC++ 2015-2022（可自动安装）
    for (id, name, arch, url) in [
        ("vc2015-2022-x64", "VC++ 2015-2022 运行库 (x64)", "x64", "https://aka.ms/vs/17/release/vc_redist.x64.exe"),
        ("vc2015-2022-x86", "VC++ 2015-2022 运行库 (x86)", "x86", "https://aka.ms/vs/17/release/vc_redist.x86.exe"),
    ] {
        let ver = vc_runtime_version(arch);
        out.push(serde_json::json!({
            "id": id, "name": name,
            "installed": ver.is_some(),
            "version": ver.unwrap_or_default(),
            "download_url": url,
        }));
    }

    // 旧版 VC++（检测真实，安装需官网下载）
    for year in ["2008", "2010", "2012", "2013"] {
        let found = detect_old_vc(year);
        for arch in ["x64", "x86"] {
            let hit = found.iter().find(|(a, _)| a == arch);
            out.push(serde_json::json!({
                "id": format!("vc{}-{}", year, arch),
                "name": format!("VC++ {} 运行库 ({})", year, arch),
                "installed": hit.is_some(),
                "version": hit.map(|(_, v)| v.clone()).unwrap_or_default(),
                "download_url": "",
            }));
        }
    }

    // .NET
    let (installed, ver) = detect_dotnet();
    out.push(serde_json::json!({
        "id": "dotnet", "name": ".NET 运行库",
        "installed": installed, "version": ver,
        "download_url": "https://dotnet.microsoft.com/download",
    }));

    out
}

/// 安装运行库：下载官方直链到临时目录（走系统代理）→ 静默安装；
/// 每 500ms/进度变化发 runtime-progress {id, stage: download|install, pct}
#[tauri::command]
pub fn install_runtime(app: tauri::AppHandle, id: String) -> Result<(), String> {
    if RUNTIME_INSTALLING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err("已有安装任务进行中，请稍候".into());
    }
    let url = match id.as_str() {
        "vc2015-2022-x64" => "https://aka.ms/vs/17/release/vc_redist.x64.exe",
        "vc2015-2022-x86" => "https://aka.ms/vs/17/release/vc_redist.x86.exe",
        _ => {
            RUNTIME_INSTALLING.store(false, std::sync::atomic::Ordering::SeqCst);
            return Err("该运行库不支持自动安装，请从官网下载安装".into());
        }
    };
    std::thread::spawn(move || {
        let _ = do_install_runtime(app, id, url);
        RUNTIME_INSTALLING.store(false, std::sync::atomic::Ordering::SeqCst);
    });
    Ok(())
}

fn do_install_runtime(app: tauri::AppHandle, id: String, url: &str) -> Result<(), String> {
    use std::io::{Read, Write};

    let emit = |stage: &str, pct: u32| {
        let _ = app.emit("runtime-progress", serde_json::json!({ "id": id, "stage": stage, "pct": pct }));
    };

    // 下载（reqwest 默认读取 HTTP_PROXY/HTTPS_PROXY 环境变量走系统代理）
    emit("download", 0);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client.get(url).send().map_err(|e| format!("下载失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let total = resp.content_length();
    let tmp = std::env::temp_dir().join(format!("jwrt-{}.exe", id));
    let mut f = std::fs::File::create(&tmp).map_err(|e| format!("创建临时文件失败: {}", e))?;
    let mut buf = [0u8; 65536];
    let mut got: u64 = 0;
    let mut last_pct: u32 = 0;
    loop {
        match resp.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                f.write_all(&buf[..n]).map_err(|e| format!("写入失败: {}", e))?;
                got += n as u64;
                if let Some(t) = total.filter(|t| *t > 0) {
                    let pct = ((got * 100 / t) as u32).min(99);
                    if pct > last_pct {
                        last_pct = pct;
                        emit("download", pct);
                    }
                }
            }
            Err(e) => return Err(format!("下载失败: {}", e)),
        }
    }
    f.flush().map_err(|e| e.to_string())?;
    drop(f);
    emit("download", 100);
    emit("install", 0);

    // 静默安装
    let status = std::process::Command::new(&tmp)
        .args(["/install", "/quiet", "/norestart"])
        .status()
        .map_err(|e| format!("无法启动安装程序: {}", e))?;
    let _ = std::fs::remove_file(&tmp);
    match status.code() {
        Some(0) | Some(1638) | Some(3010) => {
            emit("install", 100);
            Ok(())
        }
        Some(c) => Err(format!("安装失败，退出码: {}", c)),
        None => Err("安装程序被中断".into()),
    }
}

// ---------- UWP 应用管理 ----------

#[derive(Serialize)]
pub struct UwpApp {
    pub name: String,
    pub package: String,
    pub version: String,
}

/// 列出 UWP 应用（PowerShell Get-AppxPackage）
#[tauri::command]
pub async fn list_uwp() -> Result<Vec<UwpApp>, String> {
    let script = r#"[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; @(Get-AppxPackage | Select-Object Name,PackageFullName,Version) | ConvertTo-Json -Compress"#;
    let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("解析 UWP 列表失败: {}", e))?;
    let arr = match v {
        serde_json::Value::Array(a) => a,
        other => vec![other], // 单结果时 ConvertTo-Json 输出对象
    };
    Ok(arr
        .into_iter()
        .map(|x| UwpApp {
            name: x.get("Name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            package: x.get("PackageFullName").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            version: x.get("Version").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        })
        .filter(|u| !u.package.is_empty())
        .collect())
}

/// 移除 UWP 应用（PowerShell Remove-AppxPackage，按 PackageFullName）
#[tauri::command]
pub fn remove_uwp(package: String) -> Result<(), String> {
    let dirs = crate::dirs::load_dirs();
    let escaped = package.replace('\'', "''");
    let script = format!("Remove-AppxPackage -Package '{}'", escaped);
    let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output()
        .map_err(|e| e.to_string())?;
    let r = if out.status.success() {
        Ok(())
    } else {
        Err(format!("移除失败: {}", String::from_utf8_lossy(&out.stderr).trim()))
    };
    let outcome = match &r {
        Ok(_) => "成功".to_string(),
        Err(e) => format!("失败: {}", e),
    };
    crate::logs::op_log(&dirs, "uwp", "remove", &format!("移除 UWP 包 {}: {}", package, outcome));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_services_array_output() {
        // PowerShell ConvertTo-Json 正多数组输出
        let text = r#"[{"name":"wuauserv","display":"Windows Update","status":"Running","start_type":"Manual"},{"name":"Spooler","display":"Print Spooler","status":"Stopped","start_type":"Auto"}]"#;
        let list = parse_services_output(text).expect("数组输出应可解析");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "wuauserv");
        assert_eq!(list[0].display, "Windows Update");
        assert_eq!(list[0].status, "Running");
        assert_eq!(list[0].start_type, "Manual");
        assert_eq!(list[1].name, "Spooler");
        assert_eq!(list[1].status, "Stopped");
    }

    #[test]
    fn parse_services_single_object_output() {
        // 单结果时 ConvertTo-Json 输出对象（非数组）
        let text = r#"{"name":"BITS","display":"Background Intelligent Transfer Service","status":"Running","start_type":"Delayed"}"#;
        let list = parse_services_output(text).expect("单对象输出应可解析");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "BITS");
    }

    #[test]
    fn parse_services_empty_output() {
        // PowerShell 对空数组 @() 输出空串 → 应返回空列表而非报错
        assert!(parse_services_output("").unwrap().is_empty());
        assert!(parse_services_output("   \r\n").unwrap().is_empty());
    }

    #[test]
    fn parse_services_invalid_json_is_err() {
        assert!(parse_services_output("not json").is_err());
        assert!(parse_services_output("{\"truncated\":").is_err());
    }

    #[test]
    fn parse_services_drops_items_with_missing_fields() {
        // 缺字段（如缺 status）的条目被丢弃，其余保留
        let text = r#"[{"name":"ok1","display":"A","status":"Running","start_type":"Auto"},{"name":"bad","display":"B"},{"name":"ok2","display":"C","status":"Stopped","start_type":"Disabled"}]"#;
        let list = parse_services_output(text).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "ok1");
        assert_eq!(list[1].name, "ok2");
    }
}
