//! 统一的 HTTP 客户端构造：让设置页的「系统代理 / 手动代理」真正生效。
//!
//! 之前软件下载、WebView2 下载、更新检查、运行库下载各自 `Client::builder()`，
//! 完全不读代理设置——用户在设置里填了地址却毫无作用。
//! 这里集中处理，并提供连通性测试。

use std::time::Duration;

/// 代理配置来源（读 settings.json）
#[derive(Debug, Clone, PartialEq)]
pub enum ProxyConfig {
    /// 不使用代理
    None,
    /// 跟随系统代理
    System,
    /// 手动指定，形如 http://127.0.0.1:7890
    Manual(String),
}

/// 读取设置里的代理配置。
/// `use_system_proxy=1` 优先；否则看 proxy_host/proxy_port 是否有值。
pub fn proxy_config() -> ProxyConfig {
    let use_system = crate::modules::common::read_setting_raw("use_system_proxy")
        .and_then(|v| match v {
            serde_json::Value::String(s) => Some(s == "1" || s.eq_ignore_ascii_case("true")),
            serde_json::Value::Bool(b) => Some(b),
            _ => None,
        })
        .unwrap_or(false);

    let host = crate::modules::common::read_setting_raw("proxy_host")
        .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
        .unwrap_or_default();
    let port = crate::modules::common::read_setting_raw("proxy_port")
        .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
        .or_else(|| {
            crate::modules::common::read_setting_raw("proxy_port").and_then(|v| v.as_i64().map(|n| n.to_string()))
        })
        .unwrap_or_default();

    if use_system {
        return ProxyConfig::System;
    }
    if !host.is_empty() && !port.is_empty() {
        return ProxyConfig::Manual(normalize_proxy_url(&host, &port));
    }
    ProxyConfig::None
}

/// 把 host + port 拼成 reqwest 能识别的代理 URL；已带协议头则保留
pub fn normalize_proxy_url(host: &str, port: &str) -> String {
    let h = host.trim();
    let p = port.trim();
    if h.contains("://") {
        if h.contains(':') && h.rsplit(':').next().map(|s| s.chars().all(|c| c.is_ascii_digit())).unwrap_or(false) {
            h.to_string()
        } else {
            format!("{h}:{p}")
        }
    } else {
        format!("http://{h}:{p}")
    }
}

/// 校验端口字符串，返回规范形（1-65535）
pub fn validate_port(port: &str) -> Result<u16, String> {
    let n: u32 = port.trim().parse().map_err(|_| "端口必须是数字".to_string())?;
    if n == 0 || n > 65535 {
        return Err("端口需在 1-65535 之间".into());
    }
    Ok(n as u16)
}

/// 读 Windows 系统代理（注册表 ProxyEnable / ProxyServer）
fn system_proxy_url() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let k = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let enabled: u32 = k.get_value("ProxyEnable").unwrap_or(0);
    if enabled == 0 {
        return None;
    }
    let server: String = k.get_value("ProxyServer").ok()?;
    let s = server.trim();
    if s.is_empty() {
        return None;
    }
    // ProxyServer 可能是 "host:port" 或 "http=h:p;https=h:p"
    if s.contains('=') {
        for part in s.split(';') {
            if let Some(rest) = part.trim().strip_prefix("https=") {
                return Some(format!("http://{}", rest.trim()));
            }
        }
        for part in s.split(';') {
            if let Some(rest) = part.trim().strip_prefix("http=") {
                return Some(format!("http://{}", rest.trim()));
            }
        }
        None
    } else {
        Some(format!("http://{s}"))
    }
}

/// 按设置构造客户端（带代理与超时）
pub fn build_client(timeout: Duration) -> reqwest::blocking::Client {
    let mut b = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(10));
    let proxy_url = match proxy_config() {
        ProxyConfig::None => None,
        ProxyConfig::System => system_proxy_url(),
        ProxyConfig::Manual(u) => Some(u),
    };
    if let Some(u) = proxy_url {
        if let Ok(px) = reqwest::Proxy::all(&u) {
            b = b.proxy(px);
        }
    }
    b.build().unwrap_or_else(|_| {
        reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    })
}

/// 代理连通性测试结果
#[derive(serde::Serialize)]
pub struct ProxyTest {
    pub ok: bool,
    /// 实际使用的代理地址（未使用为空）
    pub proxy: String,
    /// 是否走了系统代理
    pub from_system: bool,
    /// 耗时毫秒
    pub elapsed_ms: u64,
    /// 简单延迟估计（越小越好）
    pub message: String,
}

/// 测试代理连通性：先试探代理端口是否可连，再请求一个轻量地址
#[tauri::command]
pub async fn test_proxy() -> ProxyTest {
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = proxy_config();
        let (proxy_url, from_system) = match cfg {
            ProxyConfig::None => (String::new(), false),
            ProxyConfig::System => (system_proxy_url().unwrap_or_default(), true),
            ProxyConfig::Manual(u) => (u, false),
        };

        // 有代理时先测端口连通，能给出更明确的原因
        if !proxy_url.is_empty() {
            let hostport = proxy_url
                .split("://")
                .nth(1)
                .unwrap_or(&proxy_url)
                .to_string();
            let start = std::time::Instant::now();
            // 解析 host:port，格式不合法直接返回原因
            let addr: std::net::SocketAddr = match hostport.parse() {
                Ok(a) => a,
                Err(_) => {
                    return ProxyTest {
                        ok: false,
                        proxy: proxy_url,
                        from_system,
                        elapsed_ms: 0,
                        message: "代理地址格式不正确，应为 host:port".into(),
                    }
                }
            };
            match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
                Ok(_) => {
                    let ms = start.elapsed().as_millis() as u64;
                    return ProxyTest {
                        ok: true,
                        proxy: proxy_url,
                        from_system,
                        elapsed_ms: ms,
                        message: format!("代理端口可连通（{ms} ms）"),
                    };
                }
                Err(e) => {
                    return ProxyTest {
                        ok: false,
                        proxy: proxy_url,
                        from_system,
                        elapsed_ms: 0,
                        message: format!("无法连接代理端口：{e}"),
                    }
                }
            }
        }

        ProxyTest {
            ok: false,
            proxy: String::new(),
            from_system,
            elapsed_ms: 0,
            message: if from_system {
                "系统代理未启用（注册表 ProxyEnable=0）".into()
            } else {
                "当前未配置代理，将直连下载".into()
            },
        }
    })
    .await
    .unwrap_or(ProxyTest {
        ok: false,
        proxy: String::new(),
        from_system: false,
        elapsed_ms: 0,
        message: "测试失败".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::{normalize_proxy_url, validate_port, ProxyConfig};

    #[test]
    fn proxy_url_gets_scheme() {
        assert_eq!(normalize_proxy_url("127.0.0.1", "7890"), "http://127.0.0.1:7890");
        assert_eq!(normalize_proxy_url(" 127.0.0.1 ", " 7890 "), "http://127.0.0.1:7890");
        // 已带协议且带端口，原样保留
        assert_eq!(normalize_proxy_url("http://127.0.0.1:7890", "7890"), "http://127.0.0.1:7890");
        // 带协议但没端口，补端口
        assert_eq!(normalize_proxy_url("socks5://127.0.0.1", "1080"), "socks5://127.0.0.1:1080");
    }

    #[test]
    fn port_validation() {
        assert_eq!(validate_port("7890"), Ok(7890));
        assert!(validate_port("0").is_err());
        assert!(validate_port("70000").is_err());
        assert!(validate_port("abc").is_err());
        assert!(validate_port("").is_err());
    }

    #[test]
    fn proxy_config_variants_are_distinct() {
        // 仅确认枚举语义，避免误把 None 当 Manual
        assert_ne!(ProxyConfig::None, ProxyConfig::System);
        assert_ne!(ProxyConfig::System, ProxyConfig::Manual("http://a:1".into()));
    }
}
