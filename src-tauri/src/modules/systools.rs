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

/// 禁用启动文件夹里的快捷方式：把 .lnk 移到备份目录，并记下落点与原目录
fn disable_startup_shortcut(name: &str) -> Result<(), String> {
    let folders: Vec<std::path::PathBuf> = [
        std::env::var("APPDATA").ok(),
        std::env::var("ProgramData").ok(),
    ]
    .into_iter()
    .flatten()
    .map(|p| Path::new(&p).join(r"Microsoft\Windows\Start Menu\Programs\Startup"))
    .collect();
    for folder in folders {
        let lnk = folder.join(format!("{name}.lnk"));
        if !lnk.exists() {
            continue;
        }
        let source = if folder.to_string_lossy().contains("AppData") {
            "startup-folder-user"
        } else {
            "startup-folder-all"
        };
        let stashed = crate::startup_backup::stash_shortcut(&lnk)?;
        // 用「备份路径|原目录」记录，还原时能精确放回
        crate::startup_backup::remember(
            name,
            &format!("{}|{}", stashed.to_string_lossy(), folder.to_string_lossy()),
            source,
        )?;
        return Ok(());
    }
    Err(format!("未找到快捷方式: {name}.lnk"))
}

fn toggle_startup_inner(name: String, enable: bool) -> Result<(), String> {
    if enable {
        // 优先从落盘备份还原：这是唯一能恢复「启动文件夹快捷方式」和
        // 已被删掉的 HKLM 项的方式（普通用户对 HKLM 也没有写权限）
        if let Some(b) = crate::startup_backup::take_by_name(&name) {
            return restore_from_backup(&b);
        }
        // 兼容旧流程：从本次会话的 list_startup 缓存里取命令
        let cmd = STARTUP_CACHE
            .lock()
            .ok()
            .and_then(|c| c.as_ref().and_then(|m| m.get(&name).cloned()))
            .ok_or_else(|| format!("没有找到「{}」的备份，无法还原", name))?;
        if cmd.to_lowercase().ends_with(".lnk") {
            return Err("启动文件夹快捷方式已在备份中，请点重新扫描后再试".into());
        }
        let run = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_PATH_HKCU, KEY_SET_VALUE)
            .map_err(|e| e.to_string())?;
        run.set_value(&name, &cmd).map_err(|e| e.to_string())
    } else {
        // 禁用前先记住原始命令，之后才能真的还原
        let mut last_err = None;
        for (root, path, label) in [
            (HKEY_CURRENT_USER, RUN_PATH_HKCU, "hkcu-run"),
            (HKEY_LOCAL_MACHINE, RUN_PATH_HKLM, "hklm-run"),
            (HKEY_LOCAL_MACHINE, RUN_PATH_WOW, "hklm-run-wow"),
        ] {
            let Ok(run) = RegKey::predef(root).open_subkey_with_flags(path, KEY_SET_VALUE) else {
                continue;
            };
            let cmd = match run.get_value::<String, _>(&name) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    last_err = Some(e.to_string());
                    continue;
                }
            };
            // 先落盘再删：即使删除失败，备份也不会丢
            crate::startup_backup::remember(&name, &cmd, label)?;
            match run.delete_value(&name) {
                Ok(_) => return Ok(()),
                Err(e) => last_err = Some(e.to_string()),
            }
        }
        match last_err {
            Some(e) => Err(e),
            None => {
                // Run 键里没有，可能是启动文件夹的快捷方式
                disable_startup_shortcut(&name)
            }
        }
    }
}

/// 按备份把启动项放回原处。
fn restore_from_backup(b: &crate::startup_backup::StartupBackup) -> Result<(), String> {
    match b.source.as_str() {
        "hkcu-run" => {
            let run = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(RUN_PATH_HKCU, KEY_SET_VALUE)
                .map_err(|e| format!("无法写入启动项：{e}"))?;
            run.set_value(&b.name, &b.command).map_err(|e| e.to_string())
        }
        "hklm-run" | "hklm-run-wow" => {
            // 还原系统级启动项通常需要管理员权限，失败要给出明确原因
            let path = if b.source == "hklm-run-wow" { RUN_PATH_WOW } else { RUN_PATH_HKLM };
            let run = RegKey::predef(HKEY_LOCAL_MACHINE)
                .open_subkey_with_flags(path, KEY_SET_VALUE)
                .map_err(|_| "还原系统级启动项需要管理员权限，请以管理员身份运行".to_string())?;
            run.set_value(&b.name, &b.command).map_err(|e| e.to_string())
        }
        "startup-folder-user" | "startup-folder-all" => {
            // command 存的是「备份文件路径|原目录路径」
            let (stashed, original) = b
                .command
                .split_once('|')
                .ok_or_else(|| "备份记录格式异常".to_string())?;
            crate::startup_backup::unstash_shortcut(
                std::path::Path::new(stashed),
                std::path::Path::new(original),
            )?;
            Ok(())
        }
        _ => Err("未知的启动项来源".into()),
    }
}

/// 已禁用的启动项列表（界面用来标注「已禁用」并提供还原）
#[derive(Serialize)]
pub struct DisabledStartup {
    pub name: String,
    pub command: String,
    pub source: String,
    pub disabled_at: u64,
}

#[tauri::command]
pub async fn list_disabled_startup() -> Vec<DisabledStartup> {
    crate::startup_backup::load_backups()
        .into_iter()
        .map(|b| DisabledStartup {
            name: b.name,
            command: b.command,
            source: b.source,
            disabled_at: b.disabled_at,
        })
        .collect()
}

// ---------- 登录计划任务（触发器为登录时） ----------

/// 任务名/路径白名单：字母数字、中文、空格、下划线、点、斜杠、括号、连字符。
/// 明确拒绝 `& | ; ` $`、引号和换行，避免拼进 PowerShell 单引号字面量后被打断。
fn validate_task_token(s: &str) -> bool {
    if s.trim().is_empty() {
        return false;
    }
    s.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || is_han(c)
            || matches!(c, ' ' | '_' | '.' | '/' | '\\' | '(' | ')' | '-' | '（' | '）')
    })
}

fn is_han(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c)
}

/// 单引号字面量：内部单引号加倍。调用方必须先通过 validate_task_token。
fn quote_ps_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// 固定脚本 + 已校验的单引号字面量。用户字符串不会出现在引号外。
fn build_logon_task_command(task_path: &str, task_name: &str, enabled: bool) -> Result<String, String> {
    if task_name.trim().is_empty() {
        return Err("任务名不能为空".into());
    }
    if task_path.trim().is_empty() {
        return Err("任务路径不能为空".into());
    }
    if !validate_task_token(task_path) || !validate_task_token(task_name) {
        return Err("任务路径或名称含有非法字符".into());
    }
    let verb = if enabled {
        "Enable-ScheduledTask"
    } else {
        "Disable-ScheduledTask"
    };
    Ok(format!(
        "$ErrorActionPreference='Stop'; {verb} -TaskPath {} -TaskName {}",
        quote_ps_literal(task_path),
        quote_ps_literal(task_name)
    ))
}

/// 无窗口 PowerShell：参数数组传 -Command，脚本本身由调用方固定拼好。
fn run_powershell(script: &str) -> Result<std::process::Output, String> {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.output().map_err(|e| format!("无法执行 PowerShell：{e}"))
}

/// 与本仓库其他 PowerShell 调用一致：UTF-8（可带 BOM）或无控制台时的 UTF-16 LE。
fn decode_ps_output(bytes: &[u8]) -> String {
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
    text.trim_start_matches('\u{feff}')
        .trim_matches(|c: char| c.is_whitespace() || c == '\0')
        .to_string()
}

fn ps_failure(out: &std::process::Output, action: &str) -> String {
    let err = decode_ps_output(&out.stderr);
    let err = if err.is_empty() { decode_ps_output(&out.stdout) } else { err };
    let err = if err.is_empty() { "未知错误".to_string() } else { err };
    format!("{action}：{err}")
}

fn state_label(s: &str) -> String {
    match s {
        "Ready" => "就绪".into(),
        "Disabled" => "已禁用".into(),
        "Running" => "运行中".into(),
        other => other.to_string(),
    }
}

fn json_field(v: &serde_json::Value, key: &str) -> String {
    match v.get(key) {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string().trim_matches('"').to_string(),
    }
}

/// ConvertTo-Json 单元素可能退化成对象；空输出视为空列表。最多 200 条。
fn parse_logon_tasks_json(text: &str) -> Result<Vec<serde_json::Value>, String> {
    let text = text.trim();
    if text.is_empty() || text == "null" {
        return Ok(Vec::new());
    }
    let start = text
        .find(['{', '['])
        .ok_or_else(|| "解析登录计划任务失败：输出不是 JSON".to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text[start..])
        .map_err(|e| format!("解析登录计划任务失败：{e}"))?;
    let arr = match v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    };
    let mut out = Vec::new();
    for item in arr {
        if !item.is_object() {
            continue;
        }
        let task_name = json_field(&item, "taskName");
        let task_path = json_field(&item, "taskPath");
        if task_name.is_empty() && task_path.is_empty() {
            continue;
        }
        out.push(serde_json::json!({
            "taskName": task_name,
            "taskPath": task_path,
            "state": state_label(&json_field(&item, "state")),
            "author": json_field(&item, "author"),
        }));
        if out.len() >= 200 {
            break;
        }
    }
    Ok(out)
}

/// 列出触发器为「登录时」的计划任务。
#[tauri::command]
pub async fn list_logon_tasks() -> Result<Vec<serde_json::Value>, String> {
    // 固定脚本，不含用户输入。ConvertTo-Json -InputObject @($rows) 避免单元素退化成对象。
    let script = r#"[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; $OutputEncoding=[Console]::OutputEncoding; $tasks = @(Get-ScheduledTask -ErrorAction SilentlyContinue | Where-Object { $_.Triggers | Where-Object { $_.CimClass.CimClassName -match 'Logon' } } | Select-Object -First 200); $rows = @(); foreach ($t in $tasks) { if (-not $t) { continue }; $st = [string]$t.State; $cn = switch ($st) { 'Ready' { '就绪' } 'Disabled' { '已禁用' } 'Running' { '运行中' } default { $st } }; $rows += [pscustomobject]@{ taskName = [string]$t.TaskName; taskPath = [string]$t.TaskPath; state = [string]$cn; author = [string]$t.Author } }; ConvertTo-Json -InputObject @($rows) -Compress"#;
    let out = run_powershell(script)?;
    if !out.status.success() {
        return Err(ps_failure(&out, "读取登录计划任务失败"));
    }
    parse_logon_tasks_json(&decode_ps_output(&out.stdout))
}

/// 启用或禁用一条登录计划任务。task_path / task_name 只接受白名单字符。
#[tauri::command]
pub async fn set_logon_task(task_path: String, task_name: String, enabled: bool) -> Result<(), String> {
    let script = build_logon_task_command(&task_path, &task_name, enabled)?;
    let out = run_powershell(&script)?;
    let action = if enabled { "启用" } else { "禁用" };
    let r = if out.status.success() {
        Ok(())
    } else {
        Err(ps_failure(&out, &format!("{action}登录计划任务失败")))
    };
    let dirs = crate::dirs::load_dirs();
    let outcome = match &r {
        Ok(_) => "成功".to_string(),
        Err(e) => format!("失败: {e}"),
    };
    crate::logs::op_log(
        &dirs,
        "systools",
        "set_logon_task",
        &format!("{action} {task_path}{task_name}: {outcome}"),
    );
    r
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

/// 写入 hosts（需管理员权限）。写之前把原文件复制到配置目录，便于回滚。
#[tauri::command]
pub fn write_hosts(content: String) -> Result<(), String> {
    let dirs = crate::dirs::load_dirs();
    // 先留一份原文件；备份失败不阻止写入，但会记进日志
    let backup_note = match std::fs::read(HOSTS_PATH) {
        Ok(old) => {
            let dir = std::path::Path::new(&dirs.config).join("hosts备份");
            let r = std::fs::create_dir_all(&dir).and_then(|_| {
                let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
                std::fs::write(dir.join(format!("hosts-{stamp}.txt")), &old)
            });
            match r {
                Ok(_) => "已备份原文件".to_string(),
                Err(e) => format!("原文件备份失败: {e}"),
            }
        }
        Err(e) => format!("读取原文件失败(可能是首次写入): {e}"),
    };
    let r = std::fs::write(HOSTS_PATH, &content).map_err(|e| format!("写入失败（需要管理员权限）: {}", e));
    let outcome = match &r {
        Ok(_) => "成功".to_string(),
        Err(e) => format!("失败: {}", e),
    };
    crate::logs::op_log(
        &dirs,
        "systools",
        "write_hosts",
        &format!("写入 hosts（{} 字节）: {}；{}", content.len(), outcome, backup_note),
    );
    r
}

/// hosts 里的一条记录（一行解析成一条）
#[derive(Serialize)]
pub struct HostEntry {
    pub ip: String,
    pub domains: Vec<String>,
    /// 以 # 开头视为停用
    pub enabled: bool,
    /// 仅指向 localhost 的默认示例行（界面默认折叠，避免看着像自定义记录）
    pub builtin: bool,
}

/// 系统自带示例：localhost / 127.0.0.1 / ::1 的映射，不算用户自定义
fn is_builtin_entry(domains: &[String]) -> bool {
    !domains.is_empty()
        && domains
            .iter()
            .all(|d| d.eq_ignore_ascii_case("localhost") || d.starts_with("localhost."))
}

/// 把 hosts 文本解析成结构化条目。纯函数，便于单测。
/// 说明：同一行可以有多个域名（1.2.3.4 a.com b.com），要拆开。
fn parse_hosts_entries(text: &str) -> Vec<HostEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let enabled = !trimmed.starts_with('#');
        // 去掉注释后再切字段；停用行还要去掉开头的 # 才看得到真实内容
        let body = if enabled { trimmed } else { trimmed.trim_start_matches('#') };
        let body = body.split('#').next().unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        let mut parts = body.split_whitespace();
        let Some(ip) = parts.next() else { continue };
        // 首段必须像 IP，否则是纯注释行
        if !ip.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ':') {
            continue;
        }
        let domains: Vec<String> = parts.map(|s| s.to_string()).collect();
        if domains.is_empty() {
            continue;
        }
        let builtin = is_builtin_entry(&domains);
        out.push(HostEntry {
            ip: ip.to_string(),
            domains,
            enabled,
            builtin,
        });
    }
    out
}

/// 结构化读取 hosts 条目
#[tauri::command]
pub async fn parse_hosts() -> Result<Vec<HostEntry>, String> {
    let bytes = std::fs::read(HOSTS_PATH).map_err(|e| e.to_string())?;
    Ok(parse_hosts_entries(&String::from_utf8_lossy(&bytes)))
}

/// 列出 hosts 历史备份（文件名，新的在前）
#[tauri::command]
pub async fn list_hosts_backups() -> Vec<String> {
    let dirs = crate::dirs::load_dirs();
    let dir = std::path::Path::new(&dirs.config).join("hosts备份");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "txt").unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names.reverse();
    names.truncate(30);
    names
}

/// 读取某个 hosts 备份的内容（用于预览/回滚）
#[tauri::command]
pub async fn read_hosts_backup(file: String) -> Result<String, String> {
    let name = file.trim();
    // 只接受本目录下的 .txt 文件名，防止路径穿越
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("备份文件名非法".into());
    }
    let dirs = crate::dirs::load_dirs();
    let path = std::path::Path::new(&dirs.config).join("hosts备份").join(name);
    let bytes = std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
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
    // 走统一客户端构造，运行库下载同样遵循代理设置
    let client = crate::modules::netclient::build_client(std::time::Duration::from_secs(600));
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
    let mut cmd = std::process::Command::new(&tmp);
    cmd.args(["/install", "/quiet", "/norestart"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let status = cmd
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

// ---------- 环境变量 ----------

#[derive(Serialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

fn env_hive(scope: &str) -> Result<(HKEY, &'static str), String> {
    match scope {
        "user" => Ok((HKEY_CURRENT_USER, r"Environment")),
        "system" => Ok((
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        )),
        _ => Err("scope 只能是 user 或 system".into()),
    }
}

/// 变量名校验：非空、长度、禁止 '=' / 控制字符（可单测）
fn validate_env_name(name: &str) -> Result<(), String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("变量名不能为空".into());
    }
    if n.len() > 255 {
        return Err("变量名过长（最多 255 字符）".into());
    }
    if n.chars().any(|c| c == '=' || c == '\0' || c.is_control()) {
        return Err("变量名包含非法字符".into());
    }
    Ok(())
}

/// 读取用户或系统环境变量（注册表，不展开 %VAR%）
#[tauri::command]
pub async fn get_env_vars(scope: String) -> Result<Vec<EnvVar>, String> {
    let (root, path) = env_hive(scope.trim())?;
    let key = RegKey::predef(root)
        .open_subkey(path)
        .map_err(|e| format!("打开环境变量失败: {e}"))?;
    let mut out = Vec::new();
    for (name, _) in key.enum_values().flatten() {
        let value: String = key.get_value(&name).unwrap_or_default();
        out.push(EnvVar { name, value });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

/// 写入或删除环境变量，并广播 WM_SETTINGCHANGE 让新进程生效
#[tauri::command]
pub async fn set_env_var(scope: String, name: String, value: String, is_delete: bool) -> Result<(), String> {
    validate_env_name(&name)?;
    if !is_delete && value.contains('\0') {
        return Err("变量值包含非法字符".into());
    }
    if !is_delete && value.len() > 32_767 {
        return Err("变量值过长".into());
    }
    let scope = scope.trim();
    let (root, path) = env_hive(scope)?;
    let key = RegKey::predef(root)
        .open_subkey_with_flags(path, KEY_READ | KEY_WRITE)
        .map_err(|e| {
            if scope == "system" {
                format!("写入系统环境变量失败（需要以管理员身份运行）: {e}")
            } else {
                format!("写入环境变量失败: {e}")
            }
        })?;
    let name = name.trim();
    if is_delete {
        key.delete_value(name).map_err(|e| format!("删除失败: {e}"))?;
    } else {
        key.set_value(name, &value).map_err(|e| format!("写入失败: {e}"))?;
    }
    broadcast_env_change();
    let dirs = crate::dirs::load_dirs();
    crate::logs::op_log(
        &dirs,
        "env",
        if is_delete { "delete" } else { "set" },
        &format!("{scope}\\{name}"),
    );
    Ok(())
}

/// 通知资源管理器及其他进程环境变量已变更
fn broadcast_env_change() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    let wide: Vec<u16> = std::ffi::OsStr::new("Environment")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            wide.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            1500,
            std::ptr::null_mut(),
        );
    }
}

// ---------- Path 条目化 ----------

/// 按 `;` 拆 Path。空段丢弃，不剥引号、不去空格。
fn split_path_entries(raw: &str) -> Vec<String> {
    raw.split(';').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
}

/// 用 `;` 把条目拼回 Path。空列表得到空字符串。
fn join_path_entries(entries: &[String]) -> String {
    entries.join(";")
}

fn validate_path_entry(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("Path 条目不能为空".into());
    }
    if s.contains(';') {
        return Err("Path 条目不能包含分号".into());
    }
    if s.contains('\n') || s.contains('\r') || s.contains('\0') {
        return Err("Path 条目不能包含换行".into());
    }
    if s.chars().count() >= 260 {
        return Err("Path 条目过长（须小于 260 字符）".into());
    }
    Ok(())
}

/// 新接口的 scope：user | machine。machine 对应现有系统环境变量键。
fn path_hive_scope(scope: &str) -> Result<&'static str, String> {
    match scope.trim() {
        "user" => Ok("user"),
        "machine" => Ok("system"),
        _ => Err("scope 只能是 user 或 machine".into()),
    }
}

fn read_registry_path(scope: &str) -> Result<String, String> {
    let hive = path_hive_scope(scope)?;
    let (root, sub) = env_hive(hive)?;
    let key = RegKey::predef(root)
        .open_subkey(sub)
        .map_err(|e| format!("打开环境变量失败: {e}"))?;
    match key.get_value::<String, _>("Path") {
        Ok(v) => Ok(v),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("读取 Path 失败: {e}")),
    }
}

fn path_write_error(is_machine: bool, e: &std::io::Error) -> String {
    if is_machine {
        format!("写入系统环境变量失败（需要以管理员身份运行）: {e}")
    } else {
        format!("写入环境变量失败: {e}")
    }
}

fn write_registry_path(scope: &str, value: &str) -> Result<(), String> {
    use winreg::types::ToRegValue;
    let is_machine = scope.trim() == "machine";
    let hive = path_hive_scope(scope)?;
    let (root, sub) = env_hive(hive)?;
    let key = RegKey::predef(root)
        .open_subkey_with_flags(sub, KEY_READ | KEY_WRITE)
        .map_err(|e| path_write_error(is_machine, &e))?;
    let vtype = match key.get_raw_value("Path") {
        Ok(raw) => {
            let kind = raw.vtype.clone();
            if kind != REG_SZ && kind != REG_EXPAND_SZ {
                return Err("Path 的注册表类型不是字符串，已拒绝改写".into());
            }
            kind
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => REG_EXPAND_SZ,
        Err(e) => return Err(path_write_error(is_machine, &e)),
    };
    // 只写 Path，保留原来的 REG_SZ / REG_EXPAND_SZ，不动同键下的其他变量。
    let mut raw = value.to_reg_value();
    raw.vtype = vtype;
    key.set_raw_value("Path", &raw)
        .map_err(|e| path_write_error(is_machine, &e))?;
    Ok(())
}

/// 读取用户或系统 Path，拆成 {index, value, exists}。
#[tauri::command]
pub async fn get_env_path_entries(scope: String) -> Result<Vec<serde_json::Value>, String> {
    let raw = read_registry_path(&scope)?;
    let entries = split_path_entries(&raw);
    let mut out = Vec::with_capacity(entries.len());
    for (index, value) in entries.into_iter().enumerate() {
        let exists = Path::new(&value).exists();
        out.push(serde_json::json!({
            "index": index,
            "value": value,
            "exists": exists,
        }));
    }
    Ok(out)
}

/// 用 `;` 写回 Path，并广播环境变量变更。不删除 Path 以外的变量。
#[tauri::command]
pub async fn set_env_path_entries(scope: String, entries: Vec<String>) -> Result<(), String> {
    let scope = scope.trim();
    if scope != "user" && scope != "machine" {
        return Err("scope 只能是 user 或 machine".into());
    }
    for (i, entry) in entries.iter().enumerate() {
        validate_path_entry(entry).map_err(|e| format!("第 {} 条无效：{e}", i + 1))?;
    }
    let joined = join_path_entries(&entries);
    write_registry_path(scope, &joined)?;
    broadcast_env_change();
    let dirs = crate::dirs::load_dirs();
    crate::logs::op_log(
        &dirs,
        "env",
        "set_path",
        &format!("{scope} {} 条", entries.len()),
    );
    Ok(())
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

    #[test]
    fn env_name_rejects_empty_and_illegal() {
        assert!(validate_env_name("").is_err());
        assert!(validate_env_name("   ").is_err());
        assert!(validate_env_name("A=B").is_err());
        assert!(validate_env_name("has\nline").is_err());
        assert!(validate_env_name(&"X".repeat(256)).is_err());
        assert!(validate_env_name("PATH").is_ok());
        assert!(validate_env_name(" MY_VAR ").is_ok());
    }

    #[test]
    fn hosts_entries_parse_multiline_and_disabled() {
        let text = "# 默认注释\n127.0.0.1 localhost\n\n1.2.3.4 a.com b.com  # 行尾注释\n#6.6.6.6 blocked.cn\n::1 ipv6.local\n";
        let list = parse_hosts_entries(text);
        assert_eq!(list.len(), 4, "应跳过纯注释与空行");
        assert_eq!(list[0].ip, "127.0.0.1");
        assert_eq!(list[0].domains, vec!["localhost"]);
        assert!(list[0].enabled);
        // 一行多域名要拆开
        assert_eq!(list[1].ip, "1.2.3.4");
        assert_eq!(list[1].domains, vec!["a.com", "b.com"]);
        // 注释掉的记录仍是可识别的条目，只是 enabled=false
        assert_eq!(list[2].ip, "6.6.6.6");
        assert!(!list[2].enabled);
        assert_eq!(list[3].ip, "::1");
        // 索引 0 是 127.0.0.1 localhost，属于系统示例
        assert!(list[0].builtin, "localhost 映射是系统示例");
        assert!(!list[1].builtin);
    }

    #[test]
    fn hosts_entries_ignore_prose_comments() {
        // 以 # 开头但内容不是 IP 的行不算条目
        let text = "# Copyright (c) 1993-2009 Microsoft Corp.\n# localhost name resolution\n";
        assert!(parse_hosts_entries(text).is_empty());
    }

    #[test]
    fn validate_task_token_accepts_logon_names() {
        assert!(validate_task_token(r"\Microsoft\Windows\"));
        assert!(validate_task_token("登录任务"));
        assert!(validate_task_token("My Task_1.0 (x64)"));
        assert!(validate_task_token("foo-bar/baz"));
        assert!(validate_task_token("（中文）"));
    }

    #[test]
    fn validate_task_token_rejects_empty_and_shell_chars() {
        assert!(!validate_task_token(""));
        assert!(!validate_task_token("   "));
        for bad in ["a&b", "a|b", "a;b", "a`b", "a$b", "a\nb", "a\rb", "a'b", "a\"b"] {
            assert!(!validate_task_token(bad), "{bad}");
        }
    }

    #[test]
    fn logon_task_script_embeds_only_quoted_literals() {
        let script = build_logon_task_command(r"\Microsoft\Windows\", "登录任务", true).unwrap();
        assert!(script.contains(
            r"Enable-ScheduledTask -TaskPath '\Microsoft\Windows\' -TaskName '登录任务'"
        ));
        assert!(build_logon_task_command(r"\ok\", "a&b", false).is_err());
        assert!(build_logon_task_command(r"\ok\", "", false).is_err());
        assert!(build_logon_task_command(r"\ok\", "a\nb", true).is_err());
        assert_eq!(quote_ps_literal("a'b"), "'a''b'");
    }

    #[test]
    fn parse_logon_tasks_single_object_and_cap() {
        let text = r#"{"taskName":"Sync","taskPath":"\\Foo\\","state":"Disabled","author":"NT"}"#;
        let list = parse_logon_tasks_json(text).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["state"].as_str(), Some("已禁用"));
        assert_eq!(list[0]["taskName"].as_str(), Some("Sync"));
        assert!(parse_logon_tasks_json("").unwrap().is_empty());
        assert!(parse_logon_tasks_json("null").unwrap().is_empty());
        assert!(parse_logon_tasks_json("[null]").unwrap().is_empty());
        let many = (0..205)
            .map(|i| format!(r#"{{"taskName":"T{i}","taskPath":"\\","state":"Running","author":""}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let capped = parse_logon_tasks_json(&format!("[{many}]")).unwrap();
        assert_eq!(capped.len(), 200);
        assert_eq!(capped[0]["state"].as_str(), Some("运行中"));
    }

    #[test]
    fn split_path_entries_drops_empty_keeps_quotes() {
        assert_eq!(
            split_path_entries(r#"C:\A;;"C:\Program Files";D:\bin;"#),
            vec![r"C:\A", r#""C:\Program Files""#, r"D:\bin"]
        );
        assert!(split_path_entries("").is_empty());
        assert!(split_path_entries(";;;").is_empty());
        assert_eq!(split_path_entries("a; ;b"), vec!["a", " ", "b"]);
    }

    #[test]
    fn join_path_entries_uses_semicolon() {
        let entries = vec!["C:\\A".to_string(), "D:\\B".to_string()];
        assert_eq!(join_path_entries(&entries), r"C:\A;D:\B");
        assert_eq!(join_path_entries(&[]), "");
        assert_eq!(split_path_entries(&join_path_entries(&entries)), entries);
    }

    #[test]
    fn validate_path_entry_rejects_empty_semicolon_newline_and_long() {
        assert!(validate_path_entry("").is_err());
        assert!(validate_path_entry("a;b").is_err());
        assert!(validate_path_entry("a\nb").is_err());
        assert!(validate_path_entry(&"x".repeat(260)).is_err());
        assert!(validate_path_entry(&"x".repeat(259)).is_ok());
        assert!(validate_path_entry(r"C:\Program Files").is_ok());
    }
}
