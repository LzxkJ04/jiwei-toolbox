//! 目录定位模块：dirs.json 双路径模型（program + data）读取 / 派生 / 校验 / 自动创建
//!
//! 双路径模型：
//! - program：程序安装目录（如 C:\Program Files\极维工具箱）
//! - data   ：用户数据目录（默认 %LOCALAPPDATA%\极维工具箱，可自定义）
//!   config / backup / cache / log 由 data 派生：
//!   data\配置 | data\备份导出 | data\缓存 | data\缓存\logs
//!
//! 兼容旧格式：dirs.json 无 data 字段时，data 取旧 config 的父目录；
//! 全新安装（无任何字段）时全部使用默认值。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// 六大目录（序列化为 camelCase，与前端契约一致）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dirs {
    pub program: String,
    #[serde(default)]
    pub data: String,
    pub config: String,
    pub backup: String,
    pub cache: String,
    pub log: String,
}

/// 由 data 目录派生 config/backup/cache/log
fn derive_from_data(data: &Path) -> Dirs {
    Dirs {
        program: r"C:\Program Files\极维工具箱".to_string(),
        data: data.to_string_lossy().into_owned(),
        config: data.join("配置").to_string_lossy().into_owned(),
        backup: data.join("备份导出").to_string_lossy().into_owned(),
        cache: data.join("缓存").to_string_lossy().into_owned(),
        log: data.join("缓存").join("logs").to_string_lossy().into_owned(),
    }
}

/// 默认目录：data = %LOCALAPPDATA%\极维工具箱，其余派生
pub fn default_dirs() -> Dirs {
    let la = crate::common::local_appdata();
    derive_from_data(&la.join(crate::common::APP_DIR_NAME))
}

/// 从 JSON 中按候选键取字符串（兼容旧字段名）
fn pick(v: &serde_json::Value, keys: &[&str], fallback: &str) -> String {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            if !s.trim().is_empty() {
                return s.to_string();
            }
        }
    }
    fallback.to_string()
}

/// dirs.json 候选位置：
/// 1) 旧默认配置目录（Documents\极维工具箱\配置）
/// 2) 新默认数据目录（%LOCALAPPDATA%\极维工具箱\配置）
fn dirs_json_candidates() -> Vec<PathBuf> {
    let docs = crate::common::user_documents();
    let la = crate::common::local_appdata();
    vec![
        docs.join(crate::common::APP_DIR_NAME).join("配置").join("dirs.json"),
        la.join(crate::common::APP_DIR_NAME).join("配置").join("dirs.json"),
    ]
}

/// 解析 dirs.json 文本（utf-8-sig 兼容）为 Dirs：
/// - data 缺失 → 兼容旧格式：取旧 config 的父目录；仍为空则回退 defaults.data
/// - program 等字段缺失/为空 → 取 data 派生值
/// 解析失败返回 None（调用方回退默认值）
fn parse_dirs_text(text: &str, defaults: &Dirs) -> Option<Dirs> {
    let text = crate::common::strip_bom(text.to_string());
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    // data 缺失 → 兼容旧格式：取旧 config 的父目录
    let data = pick(&v, &["data", "dataDir"], "");
    let data = if data.trim().is_empty() {
        let old_cfg = pick(&v, &["config", "configDir", "configdir"], "");
        let parent = Path::new(&old_cfg)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if parent.trim().is_empty() {
            defaults.data.clone()
        } else {
            parent
        }
    } else {
        data
    };
    let derived = derive_from_data(Path::new(&data));
    // program 保留独立存储；旧格式下若 data 派生失败且旧字段存在，尊重旧字段
    Some(Dirs {
        program: pick(&v, &["program", "programDir", "programdir"], &derived.program),
        data: derived.data,
        config: derived.config,
        backup: derived.backup,
        cache: derived.cache,
        log: derived.log,
    })
}

/// 加载目录配置：读 dirs.json（utf-8-sig），缺失/缺字段则按双路径模型派生；
/// 自动创建全部目录并写回首个候选位置（存在旧文件则原地更新）
pub fn load_dirs() -> Dirs {
    let defaults = default_dirs();
    let candidates = dirs_json_candidates();
    let json_path = candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| candidates[0].clone());

    let dirs = match fs::read(&json_path) {
        Ok(bytes) => {
            parse_dirs_text(&String::from_utf8_lossy(&bytes), &defaults)
                .unwrap_or_else(|| defaults.clone())
        }
        Err(_) => defaults.clone(),
    };

    // 自动创建目录（program 目录可能需要管理员权限，失败不阻塞）
    for d in [&dirs.data, &dirs.config, &dirs.backup, &dirs.cache, &dirs.log, &dirs.program] {
        if !d.trim().is_empty() {
            let _ = fs::create_dir_all(d);
        }
    }

    // 写回（UTF-8 无 BOM）
    if let Ok(text) = serde_json::to_string_pretty(&dirs) {
        let _ = fs::create_dir_all(json_path.parent().unwrap_or(Path::new(".")));
        let _ = fs::write(&json_path, text);
    }
    dirs
}

/// 获取六大目录
#[tauri::command]
pub fn get_dirs() -> Dirs {
    load_dirs()
}

// ---------------------------------------------------------------------------
// set_dirs：双路径自定义（校验 + 写 dirs.json + 创建目录）
// ---------------------------------------------------------------------------

/// 路径非法字符：Windows 保留字符（':' 允许作为盘符分隔，单查组件时排除）
fn has_illegal_chars(p: &str) -> bool {
    p.chars().any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
        // 除 "X:" 盘符前缀外不允许再出现冒号
        || {
            let t = p.strip_prefix(|c: char| c.is_ascii_alphabetic()).map(|r| r.starts_with(':'));
            match t {
                Some(true) => p[2..].contains(':'),
                _ => p.contains(':'),
            }
        }
}

/// 归一化路径用于比较：去掉尾部斜杠 + 小写
fn norm_path(p: &str) -> String {
    p.trim()
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// a 是否为 b 本身或其祖先目录
fn is_ancestor_or_self(a: &str, b: &str) -> bool {
    let (a, b) = (norm_path(a), norm_path(b));
    a == b || b.starts_with(&format!("{}\\", a)) || b.starts_with(&format!("{}/", a))
}

/// 校验并规范化双路径（program / data）
fn validate_dir_pair(program: &str, data: &str) -> Result<(), String> {
    for (name, p) in [("程序目录", program), ("数据目录", data)] {
        let t = p.trim();
        if t.is_empty() {
            return Err(format!("{}不能为空", name));
        }
        if has_illegal_chars(t) {
            return Err(format!("{}含非法字符（<>:\"|?*）", name));
        }
        let path = Path::new(t);
        if !path.is_absolute() {
            return Err(format!("{}必须是绝对路径", name));
        }
    }
    // 互不为父子（含相同）
    if is_ancestor_or_self(program, data) {
        return Err("数据目录不能位于程序目录内（或与之相同）".into());
    }
    if is_ancestor_or_self(data, program) {
        return Err("程序目录不能位于数据目录内（或与之相同）".into());
    }
    Ok(())
}

/// 设置程序目录与用户数据目录：
/// 校验（互不为父子、非法字符、绝对路径）→ 写 dirs.json → 创建 data 派生目录
#[tauri::command]
pub fn set_dirs(program: String, data: String) -> Result<Dirs, String> {
    validate_dir_pair(&program, &data)?;

    let data_path = PathBuf::from(data.trim());
    let mut dirs = derive_from_data(&data_path);
    dirs.program = program.trim().to_string();

    // 创建 data 派生目录 + program 目录（program 可能需管理员权限，失败不阻塞）
    for d in [&dirs.data, &dirs.config, &dirs.backup, &dirs.cache, &dirs.log] {
        fs::create_dir_all(d).map_err(|e| format!("创建目录 {} 失败: {}", d, e))?;
    }
    let _ = fs::create_dir_all(&dirs.program);

    // 写 dirs.json（新配置目录优先；旧位置存在同名文件时同步更新，避免双份配置漂移）
    let text = serde_json::to_string_pretty(&dirs).map_err(|e| e.to_string())?;
    for cand in dirs_json_candidates() {
        if cand.exists() || cand == dirs_json_candidates()[0] {
            if let Some(parent) = cand.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&cand, &text);
        }
    }
    crate::logs::op_log(&dirs, "dirs", "set_dirs", &format!("program={} data={}", dirs.program, dirs.data));
    Ok(dirs)
}

// ---------------------------------------------------------------------------
// read_legacy_dirs：旧版本注册表迁移预填
// ---------------------------------------------------------------------------

/// 读取旧版注册表目录配置 HKCU\Software\极维工具箱\Dirs（供引导壳/安装向导预填）。
/// 键不存在或全空返回 None。值名兼容中英文多种写法。
#[tauri::command]
pub fn read_legacy_dirs() -> Option<Dirs> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\极维工具箱\Dirs")
        .ok()?;
    let get = |names: &[&str]| -> String {
        for n in names {
            if let Ok(s) = key.get_value::<String, _>(n) {
                if !s.trim().is_empty() {
                    return s;
                }
            }
        }
        String::new()
    };
    let program = get(&["Program", "ProgramDir", "程序目录"]);
    let mut data = get(&["Data", "DataDir", "数据目录"]);
    let config = get(&["Config", "ConfigDir", "配置目录"]);
    let backup = get(&["Backup", "BackupDir", "备份目录"]);
    let cache = get(&["Cache", "CacheDir", "缓存目录"]);
    let log = get(&["Log", "LogDir", "日志目录"]);

    let all_empty = [&program, &data, &config, &backup, &cache, &log]
        .iter()
        .all(|s| s.trim().is_empty());
    if all_empty {
        return None;
    }

    // data 缺失时沿用旧格式兼容逻辑：取 config 父目录
    if data.trim().is_empty() {
        data = Path::new(&config)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
    }

    let mut dirs = if data.trim().is_empty() {
        let mut d = default_dirs();
        d.program = program;
        d
    } else {
        let mut d = derive_from_data(Path::new(&data));
        d.program = program;
        // 旧格式显式字段优先（尊重旧自定义）
        if !config.trim().is_empty() { d.config = config; }
        if !backup.trim().is_empty() { d.backup = backup; }
        if !cache.trim().is_empty() { d.cache = cache; }
        if !log.trim().is_empty() { d.log = log; }
        d
    };
    if dirs.program.trim().is_empty() {
        dirs.program = r"C:\Program Files\极维工具箱".to_string();
    }
    Some(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用默认值（避免依赖 %LOCALAPPDATA% 环境变量）
    fn test_defaults() -> Dirs {
        derive_from_data(Path::new(r"D:\ TestData\极维工具箱"))
    }

    // ---- dirs.json 解析（utf-8-sig / 无 BOM / 旧格式 / 缺字段） ----

    #[test]
    fn parse_with_bom() {
        // 带 UTF-8 BOM 的 utf-8-sig 文件应正常解析（JSON 内路径为转义后的反斜杠）
        let json = "\u{feff}{\"program\":\"C:\\\\App\",\"data\":\"D:\\\\Data\"}";
        let d = parse_dirs_text(json, &test_defaults()).expect("BOM JSON 应可解析");
        assert_eq!(d.program, r"C:\App");
        assert_eq!(d.data, r"D:\Data");
        assert_eq!(d.config, r"D:\Data\配置");
        assert_eq!(d.backup, r"D:\Data\备份导出");
        assert_eq!(d.cache, r"D:\Data\缓存");
        assert_eq!(d.log, r"D:\Data\缓存\logs");
    }

    #[test]
    fn parse_without_bom() {
        let json = r#"{"program":"C:\\App","data":"D:\\Data"}"#;
        let d = parse_dirs_text(json, &test_defaults()).expect("无 BOM JSON 应可解析");
        assert_eq!(d.program, r"C:\App");
        assert_eq!(d.data, r"D:\Data");
    }

    #[test]
    fn parse_legacy_format_no_data_field() {
        // 旧格式：无 data 字段，data 取旧 config 的父目录
        let json = r#"{"config":"E:\\OldDir\\配置","program":"E:\\OldDir"}"#;
        let d = parse_dirs_text(json, &test_defaults()).expect("旧格式应可解析");
        assert_eq!(d.data, r"E:\OldDir");
        assert_eq!(d.config, r"E:\OldDir\配置");
        assert_eq!(d.program, r"E:\OldDir");
    }

    #[test]
    fn parse_legacy_format_empty_config_falls_back_to_defaults() {
        // 旧格式且 config 也为空 → data 回退默认值
        let json = r#"{}"#;
        let defaults = test_defaults();
        let d = parse_dirs_text(json, &defaults).expect("空对象应可解析");
        assert_eq!(d.data, defaults.data);
        assert_eq!(d.config, defaults.config);
    }

    #[test]
    fn parse_missing_fields_use_derived_defaults() {
        // 只有 data：program 回退派生默认值，子目录全部由 data 派生
        let json = r#"{"data":"D:\\Data"}"#;
        let d = parse_dirs_text(json, &test_defaults()).unwrap();
        assert_eq!(d.program, r"C:\Program Files\极维工具箱");
        assert_eq!(d.cache, r"D:\Data\缓存");
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(parse_dirs_text("not json {", &test_defaults()).is_none());
    }

    #[test]
    fn parse_ignores_empty_string_fields() {
        // 字段为空串时视同缺失
        let json = r#"{"program":"","data":"  ","config":"E:\\OldDir\\配置"}"#;
        let d = parse_dirs_text(json, &test_defaults()).unwrap();
        // data 为空白 → 旧格式回退：config 父目录
        assert_eq!(d.data, r"E:\OldDir");
        assert_eq!(d.program, r"C:\Program Files\极维工具箱");
    }

    // ---- 路径校验（set_dirs 前置校验） ----

    #[test]
    fn illegal_chars_detection() {
        assert!(has_illegal_chars(r"C:\a<b"));
        assert!(has_illegal_chars(r"C:\a|b"));
        assert!(has_illegal_chars(r"C:\a?b*"));
        // 盘符冒号合法，第二个冒号非法
        assert!(!has_illegal_chars(r"C:\Windows"));
        assert!(has_illegal_chars(r"C:\a:b"));
        assert!(has_illegal_chars(r":foo"));
    }

    #[test]
    fn ancestor_or_self_detection() {
        assert!(is_ancestor_or_self(r"C:\A", r"C:\A"));
        assert!(is_ancestor_or_self(r"C:\A", r"C:\A\B"));
        // 尾部斜杠与大小写归一化
        assert!(is_ancestor_or_self(r"c:\a\", r"C:\A\B"));
        // 仅前缀相同但非目录边界不算祖先
        assert!(!is_ancestor_or_self(r"C:\AB", r"C:\ABC"));
        assert!(!is_ancestor_or_self(r"C:\A", r"D:\A\B"));
    }

    #[test]
    fn validate_dir_pair_rejects_bad_input() {
        // 空值
        assert!(validate_dir_pair("", r"C:\Data").is_err());
        assert!(validate_dir_pair(r"C:\Program", "  ").is_err());
        // 非绝对路径
        assert!(validate_dir_pair(r"relative\path", r"C:\Data").is_err());
        // 非法字符
        assert!(validate_dir_pair(r"C:\Pro*gram", r"C:\Data").is_err());
        // 互为父子 / 相同
        assert!(validate_dir_pair(r"C:\Same", r"C:\Same").is_err());
        assert!(validate_dir_pair(r"C:\Parent", r"C:\Parent\Data").is_err());
        assert!(validate_dir_pair(r"C:\Parent\Data", r"C:\Parent").is_err());
        // 合法组合
        assert!(validate_dir_pair(r"C:\Program Files\极维工具箱", r"D:\Data").is_ok());
    }
}


/// 修改缓存目录（log 派生为 cache 下的 logs 子目录），写回 dirs.json 并创建目录
#[tauri::command]
pub fn set_cache_dir(new_cache: String) -> Result<(), String> {
    let nc = new_cache.trim().to_string();
    if nc.is_empty()
        || nc.len() > 200
        || nc.contains("..")
        || nc.contains('/')
        || nc.contains('*')
        || nc.contains('?')
        || nc.contains('"')
        || nc.contains('<')
        || nc.contains('>')
        || nc.contains('|')
    {
        return Err("缓存目录非法".into());
    }
    let mut d = load_dirs();
    d.cache = nc.clone();
    d.log = format!("{}\\logs", nc);
    // 纯 ASCII 转义 JSON（无 BOM），写程序目录与配置目录
    let esc = |v: &str| crate::launcher::json_escape_ascii(v);
    let json = format!(
        "{{\n  \"program\": \"{}\",\n  \"data\": \"{}\",\n  \"config\": \"{}\",\n  \"backup\": \"{}\",\n  \"cache\": \"{}\",\n  \"log\": \"{}\"\n}}",
        esc(&d.program), esc(&d.data), esc(&d.config), esc(&d.backup), esc(&d.cache), esc(&d.log)
    );
    let mut wrote = false;
    let prog_json = std::path::PathBuf::from(&d.program).join("dirs.json");
    if prog_json.parent().map(|x| x.exists()).unwrap_or(false) {
        if std::fs::write(&prog_json, json.as_bytes()).is_ok() {
            wrote = true;
        }
    }
    if !wrote {
        let cfg = std::path::PathBuf::from(&d.config).join("dirs.json");
        std::fs::create_dir_all(cfg.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&cfg, json.as_bytes()).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&nc).map_err(|e| format!("创建目录失败: {}", e))?;
    Ok(())
}
