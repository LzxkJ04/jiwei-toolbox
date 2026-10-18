//! 卸载黑白名单（软件扫描过滤 + 系统保护）
//!
//! 两类规则：
//! - 卸载黑名单：命中后不在「本机软件」列表展示，无法选中卸载，防止误删系统组件
//! - 下载过滤：命中后不在软件库展示
//!
//! 匹配方式两种：
//! - name：软件名称模糊匹配（不区分大小写）
//! - path：安装路径关键词匹配
//!
//! 内置系统保护项不可删除、不可禁用，只能查看。规则存 config/settings.json。

use serde::{Deserialize, Serialize};

const KEY_RULES: &str = "uninstall_rules";
const KEY_DOWNLOAD_RULES: &str = "download_filter_rules";

/// 内置保护规则：(关键词, 匹配类型, 备注)
/// 这些是 Windows 核心组件与运行库，误删会导致系统或大量软件无法运行。
const BUILTIN: &[(&str, &str, &str)] = &[
    ("Microsoft Visual C++", "name", "VC++ 运行库，大量软件依赖"),
    ("Microsoft .NET", "name", ".NET 运行时，系统与软件依赖"),
    (".NET Framework", "name", ".NET Framework 运行时"),
    ("Windows Driver Package", "name", "驱动包，删除会导致硬件失效"),
    ("Microsoft Edge", "name", "系统内置浏览器，部分组件依赖"),
    ("Microsoft Edge WebView2", "name", "WebView2 运行时，多个应用依赖"),
    ("Windows SDK", "name", "Windows 开发组件"),
    ("Windows Software Development Kit", "name", "Windows 开发组件"),
    ("Microsoft Visual Studio Installer", "name", "VS 安装器，删除会破坏已装组件"),
    ("Microsoft Update Health Tools", "name", "更新健康组件"),
    ("Microsoft Windows Desktop Runtime", "name", ".NET 桌面运行时"),
    ("Microsoft ASP.NET Core", "name", "ASP.NET 运行时"),
    ("Windows App Runtime", "name", "WinUI / 打包应用运行时"),
    ("DirectX", "name", "图形接口运行库"),
    ("Intel(R) Chipset", "name", "芯片组驱动"),
    ("AMD Chipset", "name", "芯片组驱动"),
    ("Realtek", "name", "声卡/网卡驱动"),
    ("NVIDIA Graphics Driver", "name", "显卡驱动"),
    ("NVIDIA PhysX", "name", "物理引擎运行库"),
    ("Microsoft Corporation\\Windows", "path", "系统目录组件"),
];

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct FilterRule {
    /// 关键词
    pub keyword: String,
    /// "name" | "path"
    pub kind: String,
    #[serde(default)]
    pub note: String,
    /// 是否启用（内置规则恒为 true）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 内置规则：不可删除、不可禁用
    #[serde(default)]
    pub builtin: bool,
}

fn default_true() -> bool {
    true
}

/// 全部卸载过滤规则（内置 + 用户自定义）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleSet {
    pub uninstall: Vec<FilterRule>,
    pub download: Vec<FilterRule>,
}

fn load_rules(key: &str) -> Vec<FilterRule> {
    crate::modules::common::read_setting_raw(key)
        .and_then(|v| serde_json::from_value::<Vec<FilterRule>>(v).ok())
        .unwrap_or_default()
}

fn builtin_rules() -> Vec<FilterRule> {
    BUILTIN
        .iter()
        .map(|(kw, kind, note)| FilterRule {
            keyword: kw.to_string(),
            kind: kind.to_string(),
            note: note.to_string(),
            enabled: true,
            builtin: true,
        })
        .collect()
}

/// 读取全部规则。内置保护项每次都重新生成，保证程序升级后自动补齐。
#[tauri::command]
pub fn get_filter_rules() -> RuleSet {
    let mut uninstall = builtin_rules();
    // 用户规则里若与内置重复则跳过，避免界面出现两条一样的
    for r in load_rules(KEY_RULES) {
        if r.builtin {
            continue;
        }
        if uninstall
            .iter()
            .any(|b| b.builtin && b.keyword.eq_ignore_ascii_case(&r.keyword) && b.kind == r.kind)
        {
            continue;
        }
        uninstall.push(r);
    }
    RuleSet {
        uninstall,
        download: load_rules(KEY_DOWNLOAD_RULES),
    }
}

/// 校验单条规则
fn validate(rule: &FilterRule) -> Result<(), String> {
    let kw = rule.keyword.trim();
    if kw.is_empty() {
        return Err("关键词不能为空".into());
    }
    if kw.len() > 200 {
        return Err("关键词过长（上限 200 字符）".into());
    }
    if rule.kind != "name" && rule.kind != "path" {
        return Err("匹配类型只能是 name 或 path".into());
    }
    if kw.contains('\n') || kw.contains('\r') {
        return Err("关键词不能包含换行".into());
    }
    Ok(())
}

/// 保存用户规则（只保存非内置的，内置项由程序维护）
#[tauri::command]
pub fn save_filter_rules(uninstall: Vec<FilterRule>, download: Vec<FilterRule>) -> Result<(), String> {
    let uninstall: Vec<FilterRule> = uninstall
        .into_iter()
        .filter(|r| !r.builtin)
        .map(|mut r| {
            r.keyword = r.keyword.trim().to_string();
            r.note = r.note.trim().to_string();
            r
        })
        .collect();
    let download: Vec<FilterRule> = download
        .into_iter()
        .map(|mut r| {
            r.keyword = r.keyword.trim().to_string();
            r.note = r.note.trim().to_string();
            r.builtin = false;
            r
        })
        .collect();

    if uninstall.len() > 200 || download.len() > 200 {
        return Err("每类规则最多 200 条".into());
    }
    for r in uninstall.iter().chain(download.iter()) {
        validate(r)?;
    }

    let uv = serde_json::to_value(&uninstall).map_err(|e| e.to_string())?;
    let dv = serde_json::to_value(&download).map_err(|e| e.to_string())?;
    crate::modules::common::write_setting_raw(KEY_RULES, uv)?;
    crate::modules::common::write_setting_raw(KEY_DOWNLOAD_RULES, dv)?;

    let dirs = crate::dirs::load_dirs();
    crate::logs::op_log(
        &dirs,
        "filter",
        "save_rules",
        &format!("保存过滤规则：卸载 {} 条、下载 {} 条", uninstall.len(), download.len()),
    );
    Ok(())
}

/// 重置用户规则（内置保护项保留）
#[tauri::command]
pub fn reset_filter_rules() -> Result<(), String> {
    crate::modules::common::write_setting_raw(KEY_RULES, serde_json::json!([]))?;
    crate::modules::common::write_setting_raw(KEY_DOWNLOAD_RULES, serde_json::json!([]))?;
    Ok(())
}

/// 单条规则是否命中（供 uninstall 等模块复用）
pub fn rule_hits(rule: &FilterRule, name: &str, path: &str) -> bool {
    if !rule.enabled {
        return false;
    }
    let kw = rule.keyword.trim().to_lowercase();
    if kw.is_empty() {
        return false;
    }
    match rule.kind.as_str() {
        "path" => path.to_lowercase().contains(&kw),
        _ => name.to_lowercase().contains(&kw),
    }
}

/// 判断软件是否被下载过滤拦下。返回命中的规则（None 表示未命中）。
/// 前端拿到规则后本地过滤即可，此函数供后端内部与测试使用。
#[allow(dead_code)]
pub fn match_download(name: &str, category: &str) -> Option<FilterRule> {
    let set = get_filter_rules();
    set.download.into_iter().find(|r| rule_hits(r, name, category))
}

#[cfg(test)]
mod tests {
    use super::{builtin_rules, rule_hits, validate, FilterRule};

    fn rule(kw: &str, kind: &str, enabled: bool) -> FilterRule {
        FilterRule {
            keyword: kw.into(),
            kind: kind.into(),
            note: String::new(),
            enabled,
            builtin: false,
        }
    }

    #[test]
    fn builtin_rules_are_all_protected() {
        let list = builtin_rules();
        assert!(list.len() >= 15, "内置保护规则偏少");
        for r in &list {
            assert!(r.builtin, "内置规则必须带 builtin 标记：{}", r.keyword);
            assert!(r.enabled, "内置规则不能被禁用：{}", r.keyword);
            assert!(!r.keyword.trim().is_empty());
            assert!(r.kind == "name" || r.kind == "path");
        }
        // 关键运行库必须在保护名单里
        let kws: Vec<String> = list.iter().map(|r| r.keyword.clone()).collect();
        assert!(kws.iter().any(|k| k.contains("Visual C++")));
        assert!(kws.iter().any(|k| k.contains(".NET")));
        assert!(kws.iter().any(|k| k.contains("WebView2")));
    }

    #[test]
    fn name_match_is_case_insensitive() {
        let r = rule("visual c++", "name", true);
        assert!(rule_hits(&r, "Microsoft Visual C++ 2015-2022 Redistributable", ""));
        assert!(!rule_hits(&r, "7-Zip", ""));
    }

    #[test]
    fn path_match_uses_install_location() {
        let r = rule(r"C:\Windows", "path", true);
        assert!(rule_hits(&r, "Some App", r"c:\windows\system32\app"));
        assert!(!rule_hits(&r, "Some App", r"D:\Apps\app"));
    }

    #[test]
    fn disabled_rule_never_hits() {
        let r = rule("Notepad", "name", false);
        assert!(!rule_hits(&r, "Notepad++", ""));
        // 启用后才命中
        let r2 = rule("Notepad", "name", true);
        assert!(rule_hits(&r2, "Notepad++", ""));
    }

    #[test]
    fn empty_keyword_never_hits() {
        // 空关键词如果命中会把所有软件都过滤掉，是灾难性 bug
        let r = rule("   ", "name", true);
        assert!(!rule_hits(&r, "任意软件", "任意路径"));
    }

    #[test]
    fn validation_rejects_bad_input() {
        assert!(validate(&rule("", "name", true)).is_err());
        assert!(validate(&rule("   ", "name", true)).is_err());
        assert!(validate(&rule("ok", "unknown", true)).is_err());
        assert!(validate(&rule("a\nb", "name", true)).is_err());
        assert!(validate(&rule(&"x".repeat(201), "name", true)).is_err());
        assert!(validate(&rule("7-Zip", "name", true)).is_ok());
        assert!(validate(&rule(r"C:\Tools", "path", true)).is_ok());
    }

    /// 现场探针：内置保护规则在本机实际挡住了哪些软件。
    /// `cargo test --lib live_builtin_protection -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_builtin_protection() {
        let all = crate::modules::uninstall::list_installed_inner();
        let set = super::get_filter_rules();
        let mut protected: Vec<&str> = Vec::new();
        for a in &all {
            if set
                .uninstall
                .iter()
                .any(|r| super::rule_hits(r, &a.name, &a.install_location))
            {
                protected.push(&a.name);
            }
        }
        println!("total={} protected={}", all.len(), protected.len());
        for n in protected.iter().take(20) {
            println!("  ✓ {n}");
        }
        assert!(!protected.is_empty(), "本机应有至少一个运行库被保护");
        // 保护比例不应过高，否则用户会觉得「软件列表少了一大半」
        let ratio = protected.len() as f64 / all.len().max(1) as f64;
        println!("ratio={:.1}%", ratio * 100.0);
        assert!(ratio < 0.5, "保护比例过高，规则可能过于宽泛：{ratio:.2}");
    }
}
