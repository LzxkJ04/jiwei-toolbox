//! 更新检查：远程 version.json 查询（仅返回信息，不自动下载替换）
//!
//! 远程源：https://raw.githubusercontent.com/jiwei-toolbox/softwares/main/version.json
//! 字段：{ version, url, notes }；reqwest 默认走环境变量系统代理，5 秒超时。
//! 404 / 网络失败 → 降级为 has_update: false，latest 回退当前版本。

use serde_json::json;

const VERSION_URL: &str =
    "https://raw.githubusercontent.com/jiwei-toolbox/softwares/main/version.json";

/// 拉取远程版本信息（失败/非 2xx 返回 None）
fn fetch_version_json() -> Option<serde_json::Value> {
    let client = crate::modules::netclient::build_client(std::time::Duration::from_secs(8));
    let resp = client.get(VERSION_URL).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = resp.text().ok()?;
    serde_json::from_str(&text).ok()
}

/// 语义化版本比较：a > b（按 '.' 分段数字比较，段数不足补 0，非数字段按字符串比较）
fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (va, vb) = (parse(a), parse(b));
    let n = va.len().max(vb.len());
    for i in 0..n {
        let x = va.get(i).copied().unwrap_or(0);
        let y = vb.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// 检查更新：比较远程 version.json 与当前版本，仅返回信息
#[tauri::command]
pub async fn check_update() -> serde_json::Value {
    let current = env!("CARGO_PKG_VERSION").to_string();
    match fetch_version_json() {
        Some(v) => {
            let latest = v
                .get("version")
                .and_then(|x| x.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(&current)
                .trim()
                .to_string();
            let has_update = version_gt(&latest, &current);
            json!({
                "current": current,
                "latest": latest,
                "has_update": has_update,
                "url": v.get("url").and_then(|x| x.as_str()).unwrap_or(""),
                "notes": v.get("notes").and_then(|x| x.as_str()).unwrap_or(""),
            })
        }
        None => json!({
            "current": current,
            "latest": current,
            "has_update": false,
            "url": "",
            "notes": "检查失败（网络不可达或源未就绪）",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(version_gt("1.2.3", "1.2.2"));
        assert!(version_gt("0.2.0", "0.1.9"));
        assert!(version_gt("1.0", "0.9.9"));
        assert!(!version_gt("1.2.3", "1.2.3"));
        assert!(!version_gt("1.2", "1.2.1"));
    }
}
