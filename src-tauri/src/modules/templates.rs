//! 批量任务模板：把一套软件清单 + 执行参数存成模板，装机时一键加载。
//! 独立存放在 配置\任务模板\*.json，与当前任务队列解耦，修改模板不影响已加载的任务。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 模板内的一个软件条目（只存引用 + 该软件自己的安装参数）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TemplateItem {
    /// 软件库里的 id
    pub id: String,
    /// 冗余存名称，便于软件库里已下架时仍能展示
    #[serde(default)]
    pub name: String,
    /// 该软件单独覆盖的安装参数，空则用全局设置
    #[serde(default)]
    pub silent_args: String,
    /// 单独指定的安装路径，空则用默认
    #[serde(default)]
    pub install_dir: String,
    /// 是否跳过这个软件（保留在模板里但不安装）
    #[serde(default)]
    pub disabled: bool,
}

/// 模板的全局执行参数
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TemplateOptions {
    /// 下载完自动静默安装
    #[serde(default = "yes")]
    pub auto_install: bool,
    /// 失败策略："stop" | "skip"
    #[serde(default = "stop_policy")]
    pub fail_policy: String,
    /// 全部完成后自动关机
    #[serde(default)]
    pub auto_shutdown: bool,
    /// 加载后的行为："add" 只入队 | "start" 直接开始
    #[serde(default = "add_only")]
    pub load_action: String,
}

fn yes() -> bool {
    true
}
fn stop_policy() -> String {
    "stop".to_string()
}
fn add_only() -> String {
    "add".to_string()
}

impl Default for TemplateOptions {
    fn default() -> Self {
        Self {
            auto_install: true,
            fail_policy: stop_policy(),
            auto_shutdown: false,
            load_action: add_only(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TaskTemplate {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 标签，如「办公装机」「开发环境」
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub items: Vec<TemplateItem>,
    #[serde(default)]
    pub options: TemplateOptions,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub updated: String,
}

fn template_dir() -> Result<PathBuf, String> {
    let d = PathBuf::from(crate::dirs::load_dirs().config).join("任务模板");
    std::fs::create_dir_all(&d).map_err(|e| format!("无法创建模板目录: {e}"))?;
    Ok(d)
}

/// 模板名校验：拒绝路径穿越与非法字符
fn validate_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("模板名称不能为空".into());
    }
    if n.chars().count() > 60 {
        return Err("模板名称过长（≤60 字）".into());
    }
    if n.contains("..")
        || n.contains('/')
        || n.contains('\\')
        || n.contains(':')
        || n.contains('<')
        || n.contains('>')
        || n.contains('|')
        || n.contains('?')
        || n.contains('*')
        || n.contains('"')
    {
        return Err("模板名称含非法字符".into());
    }
    Ok(n.to_string())
}

fn template_path(name: &str) -> Result<PathBuf, String> {
    let n = validate_name(name)?;
    Ok(template_dir()?.join(format!("{n}.jwtpl.json")))
}

/// 列表项（不含完整软件清单，避免列表页传输过大）
#[derive(Serialize)]
pub struct TemplateMeta {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub count: usize,
    /// 启用（未 disabled）的软件数
    pub enabled_count: usize,
    pub created: String,
    pub updated: String,
}

#[tauri::command]
pub async fn list_templates() -> Vec<TemplateMeta> {
    tauri::async_runtime::spawn_blocking(|| {
        let Ok(dir) = template_dir() else { return vec![] };
        let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
        let mut out: Vec<TemplateMeta> = rd
            .flatten()
            .filter(|e| {
                e.path()
                    .file_name()
                    .map(|f| f.to_string_lossy().ends_with(".jwtpl.json"))
                    .unwrap_or(false)
            })
            .filter_map(|e| {
                let text = std::fs::read_to_string(e.path()).ok()?;
                let t: TaskTemplate = serde_json::from_str(&text).ok()?;
                Some(TemplateMeta {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    tags: t.tags.clone(),
                    count: t.items.len(),
                    enabled_count: t.items.iter().filter(|i| !i.disabled).count(),
                    created: t.created.clone(),
                    updated: t.updated.clone(),
                })
            })
            .collect();
        // 最近更新的排前面
        out.sort_by(|a, b| b.updated.cmp(&a.updated));
        out
    })
    .await
    .unwrap_or_default()
}

/// 读取完整模板
#[tauri::command]
pub async fn load_template(name: String) -> Result<TaskTemplate, String> {
    let path = template_path(&name)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("模板格式损坏: {e}"))
}

/// 保存模板（同名覆盖；可选新名称用于重命名）
#[tauri::command]
pub async fn save_template(template: TaskTemplate, rename_from: Option<String>) -> Result<String, String> {
    let name = validate_name(&template.name)?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut t = template;
    t.name = name.clone();
    if t.created.trim().is_empty() {
        t.created = now.clone();
    }
    t.updated = now;

    let path = template_path(&name)?;
    // 改名时把旧文件删掉，避免留下重复条目
    if let Some(old) = rename_from {
        let old = old.trim();
        if !old.is_empty() && old != name {
            let old_path = template_path(old)?;
            if old_path.exists() && old_path != path {
                let _ = std::fs::remove_file(old_path);
            }
        }
    }
    let json = serde_json::to_string_pretty(&t).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("写入失败: {e}"))?;
    crate::logs::op_log(
        &crate::dirs::load_dirs(),
        "template",
        "save",
        &format!("模板 {}（{} 个软件）", name, t.items.len()),
    );
    Ok(name)
}

/// 删除模板
#[tauri::command]
pub async fn delete_template(name: String) -> Result<(), String> {
    let path = template_path(&name)?;
    if !path.exists() {
        return Err("模板不存在".into());
    }
    std::fs::remove_file(&path).map_err(|e| format!("删除失败: {e}"))?;
    crate::logs::op_log(&crate::dirs::load_dirs(), "template", "delete", name.trim());
    Ok(())
}

/// 复制模板（生成「原名 副本」）
#[tauri::command]
pub async fn duplicate_template(name: String) -> Result<String, String> {
    let mut t = load_template(name.clone()).await?;
    let base = format!("{} 副本", t.name);
    let mut candidate = base.clone();
    let mut n = 2;
    while template_path(&candidate)?.exists() {
        candidate = format!("{base} {n}");
        n += 1;
        if n > 99 {
            return Err("副本数量过多".into());
        }
    }
    t.name = candidate.clone();
    t.created = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    save_template(t, None).await?;
    Ok(candidate)
}

/// 导出装机清单为 Markdown 文本，便于发给别人看装机方案
#[tauri::command]
pub async fn export_template_markdown(name: String) -> Result<String, String> {
    let t = load_template(name).await?;
    let mut md = String::new();
    md.push_str(&format!("# 装机方案：{}\n\n", t.name));
    if !t.description.trim().is_empty() {
        md.push_str(&format!("{}\n\n", t.description.trim()));
    }
    if !t.tags.is_empty() {
        md.push_str(&format!("标签：{}\n\n", t.tags.join(" / ")));
    }
    md.push_str(&format!("软件数量：{} 个\n\n", t.items.iter().filter(|i| !i.disabled).count()));
    md.push_str("| # | 软件 | 静默参数 | 安装目录 |\n|---|---|---|---|\n");
    for (i, it) in t.items.iter().enumerate() {
        if it.disabled {
            continue;
        }
        let args = if it.silent_args.trim().is_empty() { "默认" } else { it.silent_args.trim() };
        let dir = if it.install_dir.trim().is_empty() { "默认" } else { it.install_dir.trim() };
        md.push_str(&format!("| {} | {} | {} | {} |\n", i + 1, it.name, args, dir));
    }
    md.push_str("\n## 执行参数\n\n");
    md.push_str(&format!("- 下载后自动静默安装：{}\n", if t.options.auto_install { "是" } else { "否" }));
    md.push_str(&format!(
        "- 失败策略：{}\n",
        if t.options.fail_policy == "skip" { "跳过失败项继续" } else { "停止队列" }
    ));
    md.push_str(&format!("- 完成后自动关机：{}\n", if t.options.auto_shutdown { "是" } else { "否" }));
    Ok(md)
}

#[cfg(test)]
mod tests {
    use super::{template_dir, validate_name};

    #[test]
    fn template_name_rejects_traversal() {
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name("../etc").is_err());
        assert!(validate_name(r"a\b").is_err());
        assert!(validate_name("a:b").is_err());
        assert!(validate_name("办公装机").is_ok());
        assert!(validate_name("Dev Env 2026").is_ok());
        assert!(validate_name(&"字".repeat(61)).is_err());
    }

    #[test]
    fn template_dir_is_created() {
        // 目录应可创建（失败说明配置目录不可写）
        assert!(template_dir().is_ok());
    }
}
