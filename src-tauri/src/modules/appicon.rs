//! 软件图标离线缓存：首次在线拉取 favicon 存本地，之后永久离线可用。
//! 返回 data URL 供前端 <img> 直接渲染；并发请求按域名去重合并。

use std::collections::HashMap;
use std::sync::Mutex;
use tauri::Emitter;

use super::dirs::load_dirs;

static INFLIGHT: Mutex<Option<HashMap<String, ()>>> = Mutex::new(None);

fn icons_dir() -> Result<std::path::PathBuf, String> {
    let d = load_dirs();
    let dir = std::path::PathBuf::from(&d.cache).join("icons");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建图标缓存目录失败: {}", e))?;
    Ok(dir)
}

fn icon_path(domain: &str) -> Result<std::path::PathBuf, String> {
    let safe: String = domain
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect();
    Ok(icons_dir()?.join(format!("{}.ico", safe)))
}

fn to_data_url(bytes: &[u8]) -> String {
    use std::io::Write;
    // 手写 base64（避免新增依赖）
    const TBL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4 + 16);
    out.push_str("data:image/x-icon;base64,");
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TBL[(n >> 18) as usize & 63] as char);
        out.push(TBL[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TBL[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TBL[n as usize & 63] as char } else { '=' });
    }
    let _ = std::io::sink().write(b"");
    out
}


/// 读 Windows 系统代理（HKCU Internet Settings，如 "127.0.0.1:7890"）
fn system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let enabled: u32 = key.get_value("ProxyEnable").ok()?;
    if enabled != 1 {
        return None;
    }
    let server: String = key.get_value("ProxyServer").ok()?;
    // ProxyServer 可能形如 "127.0.0.1:7890" 或 "http=...;https=..."，取 http 段
    let server = if server.contains('=') {
        server
            .split(';')
            .find_map(|seg| seg.strip_prefix("http=").map(|s| s.to_string()))
            .unwrap_or(server)
    } else {
        server
    };
    if server.is_empty() {
        None
    } else if server.contains("://") {
        Some(server)
    } else {
        Some(format!("http://{}", server))
    }
}

/// 提取域名（复用前端同规则）
fn extract_domain(homepage: &str) -> Option<String> {
    let rest = homepage
        .trim()
        .strip_prefix("https://")
        .or_else(|| homepage.trim().strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next()?;
    if host.is_empty() {
        return None;
    }
    Some(host.trim_start_matches("www.").to_string())
}

/// 获取图标 data URL：磁盘缓存 → 在线拉取（存缓存）→ 失败空串（前端降级首字母）
#[tauri::command]
pub async fn app_icon(homepage: String) -> String {
    let Some(domain) = extract_domain(&homepage) else {
        return String::new();
    };
    let Ok(path) = icon_path(&domain) else {
        return String::new();
    };
    // 1) 磁盘缓存命中
    super::logs::op_log(&load_dirs(), "icon", "lookup", &domain);
    if let Ok(bytes) = std::fs::read(&path) {
        if !bytes.is_empty() {
            return to_data_url(&bytes);
        }
    }
    // 2) 并发去重：同一域名只允许一个在途请求
    {
        let mut guard = match INFLIGHT.lock() {
            Ok(g) => g,
            Err(_) => return String::new(),
        };
        let map = guard.get_or_insert_with(HashMap::new);
        if map.contains_key(&domain) {
            return String::new(); // 在途，前端降级首字母（下次启动有缓存）
        }
        map.insert(domain.clone(), ());
    }
    // 3) 在线拉取（走系统代理）
    let result = (|| -> Option<Vec<u8>> {
        let fetch = |client: &reqwest::blocking::Client| -> Option<Vec<u8>> {
            let resp = client
                .get(format!("https://icons.duckduckgo.com/ip3/{}.ico", domain))
                .send()
                .ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let bytes = resp.bytes().ok()?;
            if bytes.is_empty() || bytes.len() > 512 * 1024 {
                return None;
            }
            Some(bytes.to_vec())
        };
        // 1) 系统代理（注册表 ProxyEnable）
        if let Some(px) = system_proxy().and_then(|p| reqwest::Proxy::all(&p).ok()) {
            if let Ok(client) = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .proxy(px)
                .build()
            {
                if let Some(b) = fetch(&client) {
                    return Some(b);
                }
            }
        }
        // 2) 直连
        if let Ok(client) = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(6)).build() {
            if let Some(b) = fetch(&client) {
                return Some(b);
            }
        }
        // 3) 本地常见代理端口兜底（Clash/V2Ray 类，失败无害）
        for port in ["7890", "7897", "10809"] {
            if let Ok(px) = reqwest::Proxy::all(&format!("http://127.0.0.1:{}", port)) {
                if let Ok(client) = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(6)).proxy(px).build() {
                    if let Some(b) = fetch(&client) {
                        return Some(b);
                    }
                }
            }
        }
        // 4) 国内 favicon 直连源（无需代理）
        if let Ok(client) = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(6)).build() {
            if let Ok(resp) = client.get(format!("https://api.iowen.cn/favicon/{}.png", domain)).send() {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes() {
                        if !bytes.is_empty() && bytes.len() <= 512 * 1024 {
                            return Some(bytes.to_vec());
                        }
                    }
                }
            }
        }
        None
    })();
    // 4) 释放在途标记
    if let Ok(mut guard) = INFLIGHT.lock() {
        if let Some(map) = guard.as_mut() {
            map.remove(&domain);
        }
    }
    match result {
        Some(bytes) => {
            let _ = std::fs::write(&path, &bytes);
            to_data_url(&bytes)
        }
        None => String::new(),
    }
}

/// 批量预取（软件库页加载时调用一次，后台静默拉取全部图标）
#[tauri::command]
pub fn prefetch_icons(app: tauri::AppHandle, homepages: Vec<String>) {
    super::logs::op_log(&load_dirs(), "icon", "prefetch_start", &format!("count={}", homepages.len()));
    std::thread::spawn(move || {
        let mut done = 0usize;
        let total = homepages.len();
        for hp in homepages {
            let icon = app_icon(hp);
            done += 1;
            let _ = app.emit(
                "icon-prefetch",
                serde_json::json!({ "done": done, "total": total }),
            );
            let _ = icon;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_extraction() {
        assert_eq!(extract_domain("https://www.google.com/chrome/").unwrap(), "google.com");
        assert_eq!(extract_domain("http://github.com").unwrap(), "github.com");
        assert!(extract_domain("ftp://x").is_none());
        assert!(extract_domain("not a url").is_none());
    }

    #[test]
    fn data_url_format() {
        let url = to_data_url(&[0x89, 0x50, 0x4e, 0x47]);
        assert!(url.starts_with("data:image/x-icon;base64,"));
        assert!(url.contains('=') || url.len() % 4 == 0);
    }

    #[test]
    fn icon_path_sanitized() {
        let p = icon_path("a/b\\c:d").unwrap();
        let name = p.file_name().unwrap().to_string_lossy();
        assert!(!name.contains('/') && !name.contains('\\') && !name.contains(':'));
        assert!(name.ends_with(".ico"));
    }
}

#[cfg(test)]
mod net_tests {
    /// 真实网络探针（需代理环境）：cargo test -- --ignored
    #[test]
    #[ignore]
    fn probe_proxy_fetch() {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .proxy(reqwest::Proxy::all("http://127.0.0.1:7890").unwrap())
            .build()
            .unwrap();
        let resp = client
            .get("https://icons.duckduckgo.com/ip3/google.com.ico")
            .send()
            .expect("请求失败");
        assert!(resp.status().is_success());
        let bytes = resp.bytes().unwrap();
        assert!(!bytes.is_empty());
        println!("probe OK: {} bytes", bytes.len());
    }
}
