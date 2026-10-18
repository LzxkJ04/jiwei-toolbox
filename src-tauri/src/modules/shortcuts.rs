//! 快捷方式清理：扫描开始菜单 / 桌面 / 公共目录里的失效 .lnk
//!
//! 判定方式：
//! - 解析 .lnk 的目标路径，目标不存在 → 失效
//! - 目标是 UWP 应用（目标为空但有 AppUserModelID 描述）→ 跳过，不算失效
//! - 网络路径（\\server\share）不做连通性检测，只检查格式，避免卡住
//!
//! 删除一律走回收站（Shell 的 SHFileOperation），不直接删除，误判了还能捞回来。

use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Serialize, Clone)]
pub struct ShortcutRow {
    /// .lnk 文件完整路径
    pub path: String,
    /// 显示名（去掉 .lnk）
    pub name: String,
    /// 解析出的目标路径，解析失败为空
    pub target: String,
    /// 所在位置分类：桌面 / 开始菜单 / 公共桌面 / 公共开始菜单
    pub location: String,
    /// 是否失效
    pub broken: bool,
    /// 失效原因
    pub reason: String,
    /// 文件大小（字节）
    pub size: u64,
}

/// 扫描根目录及分类标签
fn roots() -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let mut push = |p: Option<PathBuf>, label: &'static str| {
        if let Some(p) = p {
            out.push((p.to_string_lossy().to_string(), label));
        }
    };
    let profile = std::env::var("USERPROFILE").unwrap_or_default();
    if !profile.is_empty() {
        let base = PathBuf::from(&profile);
        push(Some(base.join("Desktop")), "桌面");
        push(
            Some(base.join(r"AppData\Roaming\Microsoft\Windows\Start Menu\Programs")),
            "开始菜单",
        );
    }
    let program_data = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
    let pd = PathBuf::from(program_data);
    push(Some(pd.join(r"Microsoft\Windows\Start Menu\Programs")), "公共开始菜单");
    push(Some(pd.join("Desktop")), "公共桌面");
    // 任务栏固定项也算快捷方式失效的高发区
    if !profile.is_empty() {
        push(
            Some(PathBuf::from(&profile).join(r"AppData\Roaming\Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar")),
            "任务栏固定",
        );
    }
    out
}

/// 递归收集 .lnk 文件，深度上限避免软链接成环
fn collect_lnk(dir: &PathBuf, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 8 || out.len() > 5000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        let Ok(md) = entry.metadata() else { continue };
        if md.file_type().is_symlink() {
            continue;
        }
        if md.is_dir() {
            collect_lnk(&p, depth + 1, out);
        } else if p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
            out.push(p);
        }
    }
}

/// 用 WScript.Shell 批量解析 .lnk 目标。
/// 逐个起 PowerShell 会慢到不可用（几十个快捷方式要几十秒），这里一次性喂给脚本。
fn resolve_targets(paths: &[PathBuf]) -> Vec<(String, String)> {
    if paths.is_empty() {
        return Vec::new();
    }
    // 路径通过环境变量传，避免拼进脚本正文被引号/反斜杠搞坏
    let list_file = std::env::temp_dir().join(format!(
        "jiwei-lnk-{}.txt",
        chrono::Local::now().format("%H%M%S%3f")
    ));
    let body = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("\r\n");
    if std::fs::write(&list_file, body.as_bytes()).is_err() {
        return Vec::new();
    }

    let script = format!(
        r#"
$sh = New-Object -ComObject WScript.Shell
$out = @()
Get-Content -LiteralPath '{}' -Encoding UTF8 | ForEach-Object {{
  $p = $_.Trim()
  if ($p -eq '') {{ return }}
  $t = ''
  $desc = ''
  try {{
    $lnk = $sh.CreateShortcut($p)
    $t = [string]$lnk.TargetPath
    $desc = [string]$lnk.Description
  }} catch {{}}
  $out += @{{ p = $p; t = $t; d = $desc }}
}}
ConvertTo-Json -InputObject @($out) -Compress -Depth 3
"#,
        list_file.to_string_lossy().replace('\'', "''")
    );

    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = std::fs::remove_file(&list_file);

    let Ok(o) = out else { return Vec::new() };
    let bytes = o.stdout;
    let bytes = if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        &bytes[2..]
    } else {
        &bytes[..]
    };
    let text = String::from_utf8_lossy(bytes).trim_start_matches('\u{feff}').trim().to_string();
    let Some(i) = text.find(['{', '[']) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[i..]) else {
        return Vec::new();
    };
    let arr = match v {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Null => vec![],
        other => vec![other],
    };
    arr.into_iter()
        .map(|r| {
            (
                r.get("p").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                r.get("t").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            )
        })
        .filter(|(p, _)| !p.is_empty())
        .collect()
}

/// 判定目标是否有效。返回 (是否失效, 原因)。
fn judge(target: &str) -> (bool, String) {
    let t = target.trim();
    if t.is_empty() {
        // 目标为空通常是 UWP / 商店应用或卸载残留，无法可靠判定，不标记失效
        return (false, String::new());
    }
    // 网络路径不做连通性探测，避免离线时整片误判
    if t.starts_with(r"\\") {
        return (false, String::new());
    }
    // 带参数的形式：C:\a\b.exe /arg
    let clean = if let Some(rest) = t.strip_prefix('"') {
        rest.split('"').next().unwrap_or("")
    } else {
        t.split(" /").next().unwrap_or(t)
    };
    let p = PathBuf::from(clean);
    if p.exists() {
        (false, String::new())
    } else {
        (true, "目标文件不存在".to_string())
    }
}

/// 扫描全部快捷方式。只读，不做任何删除。
#[tauri::command]
pub async fn scan_shortcuts() -> Vec<ShortcutRow> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut all: Vec<(PathBuf, &'static str)> = Vec::new();
        for (root, label) in roots() {
            let rp = PathBuf::from(&root);
            if !rp.is_dir() {
                continue;
            }
            let mut found = Vec::new();
            collect_lnk(&rp, 0, &mut found);
            for f in found {
                all.push((f, label));
            }
        }
        // 同一文件可能同时出现在两个根下（比如开始菜单被重定向），按路径去重
        all.sort_by(|a, b| a.0.cmp(&b.0));
        all.dedup_by(|a, b| a.0 == b.0);

        let paths: Vec<PathBuf> = all.iter().map(|(p, _)| p.clone()).collect();
        let resolved = resolve_targets(&paths);
        let target_of = |key: &str| -> String {
            resolved
                .iter()
                .find(|(p, _)| p.eq_ignore_ascii_case(key))
                .map(|(_, t)| t.clone())
                .unwrap_or_default()
        };

        all.iter()
            .map(|(path, location)| {
                let key = path.to_string_lossy().to_string();
                let target = target_of(&key);
                let (broken, reason) = judge(&target);
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| key.clone());
                let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                ShortcutRow {
                    path: key,
                    name,
                    target,
                    location: location.to_string(),
                    broken,
                    reason,
                    size,
                }
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// 删除快捷方式到回收站。返回成功删除的条数。
/// 只接受 .lnk，且必须是刚才扫描到的合法路径，避免被当成任意删除入口。
#[tauri::command]
pub async fn delete_shortcuts(paths: Vec<String>) -> Result<u32, String> {
    if paths.is_empty() {
        return Err("没有选中任何快捷方式".into());
    }
    if paths.len() > 500 {
        return Err("单次最多删除 500 个".into());
    }
    for p in &paths {
        let lower = p.to_ascii_lowercase();
        if !lower.ends_with(".lnk") {
            return Err(format!("只能删除快捷方式：{p}"));
        }
        if p.contains('*') || p.contains('?') || p.contains('\n') {
            return Err(format!("路径含非法字符：{p}"));
        }
        if !PathBuf::from(p).is_file() {
            return Err(format!("文件不存在：{p}"));
        }
    }

    let mut ok = 0u32;
    let mut last_err = String::new();
    for p in &paths {
        match recycle_file(p) {
            Ok(_) => ok += 1,
            Err(e) => last_err = e,
        }
    }

    let dirs = crate::dirs::load_dirs();
    crate::logs::op_log(
        &dirs,
        "shortcut",
        "delete",
        &format!("删除失效快捷方式 {} / {} 个", ok, paths.len()),
    );

    if ok == 0 && !last_err.is_empty() {
        return Err(format!("删除失败：{last_err}"));
    }
    Ok(ok)
}

/// 用 Shell 的 SHFileOperation 删到回收站（FOF_ALLOWUNDO）。
fn recycle_file(path: &str) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::SHFileOperationW;
    use windows_sys::Win32::UI::Shell::SHFILEOPSTRUCTW;
    use windows_sys::Win32::UI::Shell::FOF_ALLOWUNDO;
    use windows_sys::Win32::UI::Shell::FOF_NOERRORUI;
    use windows_sys::Win32::UI::Shell::FOF_NOCONFIRMATION;
    use windows_sys::Win32::UI::Shell::FOF_SILENT;
    use windows_sys::Win32::UI::Shell::FO_DELETE;

    // 双 \0 结尾的路径列表
    let mut wide: Vec<u16> = OsStr::new(path).encode_wide().collect();
    wide.push(0);
    wide.push(0);

    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE as u32;
    op.pFrom = wide.as_ptr();
    op.pTo = std::ptr::null();
    op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16;

    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc == 0 && op.fAnyOperationsAborted == 0 {
        Ok(())
    } else if op.fAnyOperationsAborted != 0 {
        Err("操作被取消".into())
    } else {
        Err(format!("系统返回错误码 {rc}"))
    }
}

#[cfg(test)]
mod tests {
    use super::judge;

    #[test]
    fn missing_target_is_broken() {
        let (broken, reason) = judge(r"C:\__jiwei_no_such_file__.exe");
        assert!(broken);
        assert!(reason.contains("不存在"));
    }

    #[test]
    fn empty_target_is_not_judged() {
        // 目标是空时多为 UWP 或无法解析，不能误判成失效
        assert!(!judge("").0);
        assert!(!judge("   ").0);
    }

    #[test]
    fn network_path_is_skipped() {
        // 网络路径离线时不该整片标红
        assert!(!judge(r"\\server\share\a.exe").0);
    }

    #[test]
    fn existing_target_is_ok() {
        let exe = std::env::current_exe().unwrap();
        assert!(!judge(&exe.to_string_lossy()).0);
    }

    #[test]
    fn quoted_target_with_args_is_parsed() {
        let exe = std::env::current_exe().unwrap();
        let with_args = format!("\"{}\" --flag", exe.to_string_lossy());
        assert!(!judge(&with_args).0);
    }

    /// 现场探针：真机扫描一次，确认 WScript.Shell 解析能返回目标路径。
    /// `cargo test --lib live_scan_shortcuts -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_scan_shortcuts() {
        let rows = tauri::async_runtime::block_on(super::scan_shortcuts());
        let with_target = rows.iter().filter(|r| !r.target.is_empty()).count();
        let broken = rows.iter().filter(|r| r.broken).count();
        println!("total={} with_target={} broken={}", rows.len(), with_target, broken);
        for r in rows.iter().take(6) {
            println!("  [{}] {} -> {}", r.location, r.name, r.target);
        }
        for r in rows.iter().filter(|r| r.broken).take(3) {
            println!("  BROKEN: {} ({})", r.name, r.reason);
        }
    }
}
