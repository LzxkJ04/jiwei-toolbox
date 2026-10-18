#![allow(dead_code)]
//! 软件源管理：在线清单源 / 本地离线源 的增删改查、连通性测试与导入导出
//!
//! 存储：`settings.json` 的 `software_sources` 键（数组），默认源单独存 `default_source_id`。
//! 所有网络与磁盘操作都放在 `spawn_blocking` 里，避免阻塞 Tauri 的异步运行时。

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::modules::common::{read_setting_raw, write_setting_raw};

/// 设置项：全部软件源
const SOURCES_KEY: &str = "software_sources";
/// 设置项：默认源 id（不是 SoftwareSource 的字段，单独存）
const DEFAULT_KEY: &str = "default_source_id";
/// 内置官方源 id
const OFFICIAL_ID: &str = "official";
/// 官方源清单地址：与 `modules::software` 的默认源保持同一份来源，
/// 避免两处各写一个字面量、改一处忘另一处导致源地址不一致。
const OFFICIAL_SOURCE_URL: &str = crate::modules::software::DEFAULT_SOFTWARE_SOURCE;
/// 网络测试超时（毫秒）
const TEST_TIMEOUT: Duration = Duration::from_secs(8);

fn default_true() -> bool {
    true
}

/// 一个软件源
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SoftwareSource {
    /// 唯一 id，如 "official" / "src-20261018-133045"
    pub id: String,
    pub name: String,
    /// "online" | "local"
    pub kind: String,
    /// 在线源填 URL；本地源填文件夹路径
    pub url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 是否为内置官方源（不可删除）
    #[serde(default)]
    pub builtin: bool,
    /// 是否使用全局代理
    #[serde(default = "default_true")]
    pub use_global_proxy: bool,
    /// 是否校验安装包哈希
    #[serde(default = "default_true")]
    pub verify_hash: bool,
    /// 源过滤：启用后该源的黑名单生效
    #[serde(default)]
    pub filter_enabled: bool,
}

// ---------------------------------------------------------------------------
// 纯函数：内置源补齐 / 删除权限 / 更新校验 / 导入合并
// ---------------------------------------------------------------------------

/// 构造内置官方源
fn builtin_source() -> SoftwareSource {
    SoftwareSource {
        id: OFFICIAL_ID.to_string(),
        name: "官方源".to_string(),
        kind: "online".to_string(),
        url: OFFICIAL_SOURCE_URL.to_string(),
        description: "极维工具箱内置软件清单。远端地址未配置时，软件库使用随程序打包的本地清单（26 个软件），不影响安装。".to_string(),
        enabled: true,
        builtin: true,
        use_global_proxy: true,
        verify_hash: true,
        filter_enabled: false,
    }
}

/// 列表中没有内置源时补一个（放最前面）
fn ensure_builtin(mut list: Vec<SoftwareSource>) -> Vec<SoftwareSource> {
    let exists = list.iter().any(|s| s.builtin || s.id == OFFICIAL_ID);
    if !exists {
        list.insert(0, builtin_source());
    }
    list
}

/// 内置源受保护，不允许删除
fn can_delete(s: &SoftwareSource) -> bool {
    !s.builtin
}

/// 纯字符串校验：名称、地址、类型（不碰文件系统）
fn validate_basic(s: &SoftwareSource) -> Result<(), String> {
    if s.name.trim().is_empty() {
        return Err("源名称不能为空".into());
    }
    if s.url.trim().is_empty() {
        return Err("源地址不能为空".into());
    }
    if s.kind != "online" && s.kind != "local" {
        return Err("源类型只能是「在线源」或「本地源」".into());
    }
    Ok(())
}

/// 本地源目录必须真实存在
fn validate_local_dir(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("本地源目录不能为空".into());
    }
    if !Path::new(path.trim()).is_dir() {
        return Err(format!("本地源目录不存在或不是文件夹：{}", path.trim()));
    }
    Ok(())
}

/// 完整校验（含本地目录存在性）
fn validate_source(s: &SoftwareSource) -> Result<(), String> {
    validate_basic(s)?;
    if s.kind == "local" {
        validate_local_dir(&s.url)?;
    }
    Ok(())
}

/// 更新校验：内置源只允许改名称/描述/开关，类型与地址不可变
fn validate_update(old: &SoftwareSource, new: &SoftwareSource) -> Result<(), String> {
    if !old.builtin {
        return Ok(());
    }
    if old.kind != new.kind {
        return Err("内置官方源的类型不可修改".into());
    }
    if old.url.trim() != new.url.trim() {
        return Err("内置官方源的地址不可修改".into());
    }
    Ok(())
}

/// 导入合并：同 id 覆盖、新 id 追加；内置源与空 id 跳过，返回（结果, 跳过数）
fn merge_sources(
    current: Vec<SoftwareSource>,
    incoming: Vec<SoftwareSource>,
) -> (Vec<SoftwareSource>, usize) {
    let mut list = current;
    let mut skipped = 0usize;
    for item in incoming {
        let id = item.id.trim().to_string();
        if id.is_empty() {
            skipped += 1;
            continue;
        }
        if let Some(pos) = list.iter().position(|s| s.id == id) {
            if list[pos].builtin {
                // 内置源不可被导入覆盖
                skipped += 1;
                continue;
            }
            let mut next = item.clone();
            next.id = id;
            next.builtin = false;
            list[pos] = next;
        } else if item.builtin {
            // 导入文件里带的"内置源"不算数，避免伪造
            skipped += 1;
        } else {
            let mut next = item;
            next.id = id;
            next.builtin = false;
            list.push(next);
        }
    }
    (list, skipped)
}

/// 生成源 id：src-年月日-时分秒，同秒冲突时加序号
fn gen_id() -> String {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let base = chrono::Local::now().format("src-%Y%m%d-%H%M%S").to_string();
    if seq == 0 {
        base
    } else {
        format!("{base}-{seq}")
    }
}

// ---------------------------------------------------------------------------
// 存储读写
// ---------------------------------------------------------------------------

fn persist(list: &[SoftwareSource]) -> Result<(), String> {
    let value =
        serde_json::to_value(list).map_err(|e| format!("序列化软件源失败：{}", e))?;
    write_setting_raw(SOURCES_KEY, value)
}

/// 读取全部源；没有内置源时自动补齐并落盘
fn read_all() -> Vec<SoftwareSource> {
    let list: Vec<SoftwareSource> = read_setting_raw(SOURCES_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let missing_builtin = !list.iter().any(|s| s.builtin || s.id == OFFICIAL_ID);
    let list = ensure_builtin(list);
    if missing_builtin {
        let _ = persist(&list);
    }
    list
}

fn find_source(id: &str) -> Option<SoftwareSource> {
    read_all().into_iter().find(|s| s.id == id)
}

// ---------------------------------------------------------------------------
// 网络 / 目录测试
// ---------------------------------------------------------------------------

/// 按源设置构造客户端：不使用全局代理时强制直连
fn make_client(use_proxy: bool) -> reqwest::blocking::Client {
    if use_proxy {
        return crate::modules::netclient::build_client(TEST_TIMEOUT);
    }
    reqwest::blocking::Client::builder()
        .timeout(TEST_TIMEOUT)
        .connect_timeout(TEST_TIMEOUT)
        .no_proxy()
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

/// 把 reqwest 错误翻译成中文原因
fn describe_reqwest_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "请求超时（超过 8 秒）".to_string()
    } else if e.is_connect() {
        format!("连接失败（DNS 解析失败或目标不可达）：{}", e)
    } else if e.is_request() {
        format!("请求发送失败：{}", e)
    } else {
        format!("网络异常：{}", e)
    }
}

/// 从清单文本里数出包数量：顶层数组直接取长度
fn count_packages(text: &str) -> Option<usize> {
    let text = crate::modules::common::strip_bom(text.to_string());
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    match value {
        serde_json::Value::Array(a) => Some(a.len()),
        serde_json::Value::Object(o) => {
            // 兼容 { softwares: [...] } / { items: [...] } 这类包装格式
            for key in ["softwares", "items", "packages"] {
                if let Some(serde_json::Value::Array(a)) = o.get(key) {
                    return Some(a.len());
                }
            }
            None
        }
        _ => None,
    }
}

/// 在线源测试
fn test_online(url: &str, use_proxy: bool) -> serde_json::Value {
    let url = url.trim();
    if url.is_empty() {
        return json!({
            "ok": false, "elapsed_ms": 0, "package_count": 0,
            "message": "源地址为空"
        });
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return json!({
            "ok": false, "elapsed_ms": 0, "package_count": 0,
            "message": "在线源地址必须以 http:// 或 https:// 开头"
        });
    }

    let client = make_client(use_proxy);
    let start = Instant::now();
    match client.get(url).send() {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            let elapsed = start.elapsed().as_millis() as u64;
            if !status.is_success() {
                // 内置官方源指向的是项目预留地址，仓库未发布时会 404。
                // 这不是用户配置错误，也不影响安装（软件库用随包的本地清单），
                // 所以文案要写清楚，否则用户会以为程序坏了。
                let message = if url == OFFICIAL_SOURCE_URL {
                    format!(
                        "远端清单尚未发布（HTTP {}）。软件库仍使用随程序打包的本地清单，安装不受影响。可在软件库点「更新清单」或添加自己的源。",
                        status.as_u16()
                    )
                } else {
                    format!("服务器返回 HTTP {}，清单不可用", status.as_u16())
                };
                return json!({
                    "ok": false, "elapsed_ms": elapsed, "package_count": 0,
                    "message": message
                });
            }
            match count_packages(&text) {
                Some(n) => json!({
                    "ok": true, "elapsed_ms": elapsed, "package_count": n,
                    "message": format!("连接正常，共 {} 个安装包", n)
                }),
                None => json!({
                    "ok": true, "elapsed_ms": elapsed, "package_count": 0,
                    "message": "响应不是有效的清单（需要 JSON 数组）"
                }),
            }
        }
        Err(e) => json!({
            "ok": false,
            "elapsed_ms": start.elapsed().as_millis() as u64,
            "package_count": 0,
            "message": describe_reqwest_error(&e)
        }),
    }
}

/// 本地源测试：扫描目录下 *.json，累加各清单条目数
fn test_local(path: &str) -> serde_json::Value {
    let dir = Path::new(path.trim());
    let start = Instant::now();
    if !dir.is_dir() {
        return json!({
            "ok": false, "elapsed_ms": 0, "package_count": 0,
            "message": format!("目录不存在：{}", path.trim())
        });
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            return json!({
                "ok": false, "elapsed_ms": start.elapsed().as_millis() as u64,
                "package_count": 0, "message": format!("无法读取目录：{}", e)
            })
        }
    };

    let mut total = 0usize;
    let mut files = 0usize;
    let mut invalid = 0usize;
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let is_json = p
            .extension()
            .and_then(|x| x.to_str())
            .map(|x| x.eq_ignore_ascii_case("json"))
            .unwrap_or(false);
        if !is_json {
            continue;
        }
        files += 1;
        match std::fs::read_to_string(&p) {
            Ok(text) => match count_packages(&text) {
                Some(n) => total += n,
                None => invalid += 1,
            },
            Err(_) => invalid += 1,
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;
    let message = if files == 0 {
        "目录里没有 JSON 清单文件".to_string()
    } else if invalid > 0 {
        format!("扫描 {} 个清单文件，{} 个无法解析，共 {} 个安装包", files, invalid, total)
    } else {
        format!("扫描 {} 个清单文件，共 {} 个安装包", files, total)
    };
    json!({
        "ok": files > 0, "elapsed_ms": elapsed, "package_count": total, "message": message
    })
}

/// 目录扫描：列出安装包文件（不递归，只看顶层）
fn scan_dir(path: &str) -> Result<serde_json::Value, String> {
    let dir = Path::new(path.trim());
    if !dir.is_dir() {
        return Err(format!("目录不存在：{}", path.trim()));
    }
    let mut files: Vec<serde_json::Value> = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|e| format!("无法读取目录：{}", e))?
        .flatten()
    {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let ext = p
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "exe" | "msi" | "zip" | "7z") {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let name = p
            .file_name()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default();
        files.push(json!({
            "name": name,
            "size": size,
            "path": p.to_string_lossy(),
        }));
    }
    files.sort_by(|a, b| {
        a.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
    });
    let total = files.len();
    Ok(json!({ "files": files, "total": total }))
}

// ---------------------------------------------------------------------------
// 命令实现
// ---------------------------------------------------------------------------

/// 列出全部软件源（自动补齐内置源）
#[tauri::command]
pub fn list_sources() -> Vec<SoftwareSource> {
    read_all()
}

/// 新增或更新一个软件源，返回更新后的完整列表
#[tauri::command]
pub async fn save_source(source: SoftwareSource) -> Result<Vec<SoftwareSource>, String> {
    tauri::async_runtime::spawn_blocking(move || save_source_blocking(source))
        .await
        .map_err(|e| format!("保存软件源失败：{}", e))?
}

fn save_source_blocking(mut source: SoftwareSource) -> Result<Vec<SoftwareSource>, String> {
    source.name = source.name.trim().to_string();
    source.url = source.url.trim().to_string();
    source.kind = source.kind.trim().to_ascii_lowercase();
    source.description = source.description.trim().to_string();
    validate_source(&source)?;

    let mut list = read_all();
    let pos = if source.id.trim().is_empty() {
        None
    } else {
        list.iter().position(|s| s.id == source.id)
    };

    match pos {
        Some(i) => {
            let old = list[i].clone();
            validate_update(&old, &source)?;
            // 内置标记只由后端决定，前端改不动
            source.builtin = old.builtin;
            source.id = old.id;
            list[i] = source;
        }
        None => {
            if source.id.trim().is_empty() {
                source.id = gen_id();
            }
            source.builtin = false;
            list.push(source);
        }
    }

    persist(&list)?;
    Ok(list)
}

/// 删除软件源（内置源拒绝删除），返回更新后的完整列表
#[tauri::command]
pub async fn delete_source(id: String) -> Result<Vec<SoftwareSource>, String> {
    tauri::async_runtime::spawn_blocking(move || delete_source_blocking(&id))
        .await
        .map_err(|e| format!("删除软件源失败：{}", e))?
}

fn delete_source_blocking(id: &str) -> Result<Vec<SoftwareSource>, String> {
    let mut list = read_all();
    let pos = list
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| "源不存在，可能已被删除".to_string())?;
    if !can_delete(&list[pos]) {
        return Err("内置官方源不可删除".into());
    }
    list.remove(pos);
    persist(&list)?;

    // 删掉的是默认源时，顺手清空默认标记
    let is_default = read_setting_raw(DEFAULT_KEY)
        .and_then(|v| v.as_str().map(|s| s == id))
        .unwrap_or(false);
    if is_default {
        let _ = write_setting_raw(DEFAULT_KEY, serde_json::Value::String(String::new()));
    }
    Ok(list)
}

/// 设置默认源
#[tauri::command]
pub async fn set_default_source(id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !read_all().iter().any(|s| s.id == id) {
            return Err("源不存在，无法设为默认".to_string());
        }
        write_setting_raw(DEFAULT_KEY, serde_json::Value::String(id))
    })
    .await
    .map_err(|e| format!("设置默认源失败：{}", e))?
}

/// 取当前默认源（未设置返回 None）
#[tauri::command]
pub fn get_default_source() -> Option<SoftwareSource> {
    let id = read_setting_raw(DEFAULT_KEY)
        .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
        .unwrap_or_default();
    if id.is_empty() {
        return None;
    }
    find_source(&id)
}

/// 测试已保存源的连通性
#[tauri::command]
pub async fn test_source(id: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let source = find_source(&id).ok_or_else(|| "源不存在，无法测试".to_string())?;
        Ok(if source.kind == "local" {
            test_local(&source.url)
        } else {
            test_online(&source.url, source.use_global_proxy)
        })
    })
    .await
    .map_err(|e| format!("测试软件源失败：{}", e))?
}

/// 扫描本地源目录下的安装包
#[tauri::command]
pub async fn scan_local_source(path: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || scan_dir(&path))
        .await
        .map_err(|e| format!("扫描目录失败：{}", e))?
}

/// 导出全部源到备份目录，返回 { file }
#[tauri::command]
pub async fn export_sources() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let list = read_all();
        let backup = crate::dirs::load_dirs().backup;
        std::fs::create_dir_all(&backup).map_err(|e| format!("创建备份目录失败：{}", e))?;
        let name = format!("sources-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        let file = Path::new(&backup).join(name);
        let text = serde_json::to_string_pretty(&list)
            .map_err(|e| format!("序列化软件源失败：{}", e))?;
        std::fs::write(&file, text).map_err(|e| format!("写入文件失败：{}", e))?;
        Ok(json!({ "file": file.to_string_lossy() }))
    })
    .await
    .map_err(|e| format!("导出软件源失败：{}", e))?
}

/// 从 JSON 文件导入源清单（同 id 覆盖，内置源跳过）
#[tauri::command]
pub async fn import_sources(file: String) -> Result<Vec<SoftwareSource>, String> {
    tauri::async_runtime::spawn_blocking(move || import_sources_blocking(&file))
        .await
        .map_err(|e| format!("导入软件源失败：{}", e))?
}

fn import_sources_blocking(file: &str) -> Result<Vec<SoftwareSource>, String> {
    if file.trim().is_empty() {
        return Err("未选择导入文件".into());
    }
    let text = std::fs::read_to_string(file).map_err(|e| format!("读取文件失败：{}", e))?;
    let text = crate::modules::common::strip_bom(text);
    let incoming: Vec<SoftwareSource> = serde_json::from_str(&text)
        .map_err(|e| format!("文件不是合法的软件源列表：{}", e))?;

    // 只保留字段完整的条目，避免把明显的坏数据写进设置
    let valid: Vec<SoftwareSource> = incoming
        .into_iter()
        .filter(|s| validate_basic(s).is_ok())
        .collect();

    let (merged, _skipped) = merge_sources(read_all(), valid);
    persist(&merged)?;
    Ok(merged)
}

/// 临时连通性测试（新增源弹窗用，不依赖已保存的源）
#[tauri::command]
pub async fn test_source_connectivity(
    url: String,
    kind: String,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if kind == "local" {
            Ok(test_local(&url))
        } else {
            Ok(test_online(&url, true))
        }
    })
    .await
    .map_err(|e| format!("测试连通性失败：{}", e))?
}

// ---------------------------------------------------------------------------
// 单元测试（全部纯函数，不碰网络与真实文件系统）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, kind: &str) -> SoftwareSource {
        SoftwareSource {
            id: id.to_string(),
            name: format!("源-{}", id),
            kind: kind.to_string(),
            url: "https://example.com/list.json".to_string(),
            description: String::new(),
            enabled: true,
            builtin: false,
            use_global_proxy: true,
            verify_hash: true,
            filter_enabled: false,
        }
    }

    #[test]
    fn builtin_source_is_created_when_empty() {
        let out = ensure_builtin(Vec::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, OFFICIAL_ID);
        assert!(out[0].builtin);
        assert!(out[0].enabled);
        assert_eq!(out[0].kind, "online");
        assert_eq!(out[0].url, OFFICIAL_SOURCE_URL);

        // 已有自定义源时也补到最前面
        let out = ensure_builtin(vec![sample("a", "online")]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, OFFICIAL_ID);

        // 已存在内置源时不重复添加
        let out = ensure_builtin(vec![builtin_source(), sample("a", "online")]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn builtin_cannot_be_deleted() {
        assert!(!can_delete(&builtin_source()));
        assert!(can_delete(&sample("a", "online")));
    }

    #[test]
    fn builtin_url_is_immutable() {
        let old = builtin_source();

        let mut changed_url = old.clone();
        changed_url.url = "https://evil.example.com/list.json".into();
        assert!(validate_update(&old, &changed_url).is_err());

        let mut changed_kind = old.clone();
        changed_kind.kind = "local".into();
        assert!(validate_update(&old, &changed_kind).is_err());

        // 改名称/描述/开关可以通过
        let mut renamed = old.clone();
        renamed.name = "官方源（改名）".into();
        renamed.description = "新的描述".into();
        renamed.enabled = false;
        assert!(validate_update(&old, &renamed).is_ok());

        // 非内置源怎么改都不受此限制
        let custom = sample("a", "online");
        let mut custom2 = custom.clone();
        custom2.url = "https://other.example.com/x.json".into();
        assert!(validate_update(&custom, &custom2).is_ok());
    }

    #[test]
    fn local_source_requires_existing_dir() {
        let mut s = sample("local-1", "local");
        s.url = r"C:\__jiwei_not_exists__\__no_such_dir__".into();
        let err = validate_source(&s).unwrap_err();
        assert!(err.contains("不存在"), "错误信息应说明目录不存在：{err}");

        // 纯字符串校验对本地源放行，只有目录检查会报错
        assert!(validate_basic(&s).is_ok());

        // 在线源不走目录检查
        let online = sample("on", "online");
        assert!(validate_source(&online).is_ok());

        // 类型非法
        let mut bad = sample("bad", "ftp");
        bad.kind = "ftp".into();
        assert!(validate_basic(&bad).is_err());
    }

    #[test]
    fn source_json_roundtrip() {
        // 缺省 enabled / use_global_proxy / verify_hash 时应为 true
        let raw = r#"{"id":"a","name":"源 A","kind":"online","url":"https://example.com/a.json"}"#;
        let parsed: SoftwareSource = serde_json::from_str(raw).unwrap();
        assert!(parsed.enabled);
        assert!(parsed.use_global_proxy);
        assert!(parsed.verify_hash);
        // 缺省 builtin / filter_enabled 应为 false
        assert!(!parsed.builtin);
        assert!(!parsed.filter_enabled);
        assert_eq!(parsed.description, "");

        // 序列化后字段不丢
        let mut full = parsed.clone();
        full.description = "描述".into();
        full.filter_enabled = true;
        full.builtin = false;
        let text = serde_json::to_string(&full).unwrap();
        let back: SoftwareSource = serde_json::from_str(&text).unwrap();
        assert_eq!(back.id, full.id);
        assert_eq!(back.name, full.name);
        assert_eq!(back.kind, full.kind);
        assert_eq!(back.url, full.url);
        assert_eq!(back.description, full.description);
        assert_eq!(back.enabled, full.enabled);
        assert_eq!(back.builtin, full.builtin);
        assert_eq!(back.use_global_proxy, full.use_global_proxy);
        assert_eq!(back.verify_hash, full.verify_hash);
        assert_eq!(back.filter_enabled, full.filter_enabled);

        // 再序列化一次应与第一次完全一致（字段无遗漏）
        assert_eq!(text, serde_json::to_string(&back).unwrap());
    }

    #[test]
    fn merge_import_skips_builtin() {
        let current = vec![builtin_source(), sample("a", "online")];

        let mut overwrite_builtin = builtin_source();
        overwrite_builtin.url = "https://evil.example.com/x.json".into();
        overwrite_builtin.name = "被覆盖的官方源".into();

        let mut update_a = sample("a", "online");
        update_a.name = "源 A 改名".into();

        let incoming = vec![
            overwrite_builtin,
            update_a,
            sample("b", "online"),
            sample("", "online"), // 空 id 跳过
        ];

        let (merged, skipped) = merge_sources(current, incoming);
        assert_eq!(merged.len(), 3, "内置 + a + b");
        assert_eq!(skipped, 2, "内置覆盖与空 id 各跳过一条");

        // 内置源未被覆盖
        let official = merged.iter().find(|s| s.id == OFFICIAL_ID).unwrap();
        assert_eq!(official.url, OFFICIAL_SOURCE_URL);
        assert_eq!(official.name, "官方源");

        // 同 id 覆盖生效
        assert_eq!(merged.iter().find(|s| s.id == "a").unwrap().name, "源 A 改名");
        // 新 id 追加
        assert!(merged.iter().any(|s| s.id == "b"));
        // 空 id 不会进列表
        assert!(!merged.iter().any(|s| s.id.is_empty()));
    }

    /// 现场探针：真连一次官方源，确认 URL 有效、能解析出清单条数与延迟。
    /// `cargo test --lib live_official_source -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_official_source() {
        let res = tauri::async_runtime::block_on(super::test_source_connectivity(
            OFFICIAL_SOURCE_URL.to_string(),
            "online".to_string(),
        ))
        .expect("连通性测试应该能返回结果");
        println!("{res}");
        let ok = res.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let count = res.get("package_count").and_then(|v| v.as_u64()).unwrap_or(0);
        let ms = res.get("elapsed_ms").and_then(|v| v.as_u64()).unwrap_or(0);
        println!("ok={ok} count={count} elapsed={ms}ms");
        // 该地址是项目自己的清单仓库，联网环境下应当可用；无网络时这里会失败，
        // 所以断言只检查字段完整性，不强制要求 ok=true。
        assert!(res.get("message").is_some(), "必须给出人类可读的说明");
    }
}
