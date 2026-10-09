//! 代理探测：默认跟随 Windows 系统代理（注册表 Internet Settings），
//! 其次读取环境变量，最后允许在界面里手动覆盖。

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const IE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

#[derive(Debug, Clone, Default)]
pub struct ProxyInfo {
    /// 规范化后的代理地址，例如 http://127.0.0.1:7897；None 表示直连
    pub url: Option<String>,
    /// 人类可读的来源说明，直接显示在界面上
    pub desc: String,
    /// 是否检测到 PAC 自动配置脚本（脚本内容无法本地解析，只能提示）
    pub pac: Option<String>,
}

/// 按 环境变量 → 注册表 的顺序探测系统代理
pub fn detect() -> ProxyInfo {
    if let Some((url, src)) = from_env() {
        return ProxyInfo {
            url: Some(url),
            desc: src,
            pac: None,
        };
    }
    from_registry()
}

fn from_env() -> Option<(String, String)> {
    for key in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        if let Ok(v) = std::env::var(key) {
            if !v.trim().is_empty() {
                if let Some(u) = normalize_server(&v) {
                    return Some((u, format!("环境变量 {key}")));
                }
            }
        }
    }
    None
}

fn from_registry() -> ProxyInfo {
    let k = match RegKey::predef(HKEY_CURRENT_USER).open_subkey(IE_KEY) {
        Ok(k) => k,
        Err(e) => {
            return ProxyInfo {
                url: None,
                desc: format!("无法读取系统代理设置（{e}）"),
                pac: None,
            }
        }
    };
    let enable: u32 = k.get_value("ProxyEnable").unwrap_or(0);
    let server: String = k.get_value("ProxyServer").unwrap_or_default();
    let pac: String = k.get_value("AutoConfigURL").unwrap_or_default();

    if enable == 1 && !server.trim().is_empty() {
        if let Some(url) = normalize_server(&server) {
            return ProxyInfo {
                url: Some(url),
                desc: "Windows 系统代理（注册表）".to_string(),
                pac: none_if_empty(&pac),
            };
        }
    }
    let mut desc = if !pac.trim().is_empty() {
        format!("系统使用 PAC 自动代理，无法自动解析，请手动填写代理地址或走直连")
    } else if enable == 1 {
        "系统代理已启用但未填写地址，走直连".to_string()
    } else {
        "系统未启用代理，走直连".to_string()
    };
    if server.trim().is_empty() == false && enable != 1 {
        desc = format!("系统代理已关闭（登记的地址：{server}）");
    }
    ProxyInfo {
        url: None,
        desc,
        pac: none_if_empty(&pac),
    }
}

fn none_if_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// 把 "127.0.0.1:7897" / "http=1.2.3.4:80;https=5.6.7.8:443" / "socks5://host:port"
/// 统一成 ureq 可用的形式
pub fn normalize_server(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // 形如 http=a:8080;https=b:8080;socks=c:1080;no_proxy=...
    if s.contains('=') {
        let mut picked: Option<String> = None;
        for part in s.split(';') {
            let mut kv = part.splitn(2, '=');
            let (k, v) = (kv.next()?.trim().to_ascii_lowercase(), kv.next()?.trim());
            if v.is_empty() || k == "no_proxy" {
                continue;
            }
            if k == "https" || k == "http" || k == "ftp" {
                picked = Some(v.to_string());
                if k == "https" {
                    break;
                }
            } else if k == "socks" || k == "socks5" {
                picked.get_or_insert_with(|| v.to_string());
            }
        }
        return picked.and_then(|p| wrap_scheme(&p));
    }
    wrap_scheme(s)
}

fn wrap_scheme(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("socks5://")
        || lower.starts_with("socks4://")
    {
        return Some(t.to_string());
    }
    Some(format!("http://{t}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_host_port() {
        assert_eq!(normalize_server("127.0.0.1:7897").as_deref(), Some("http://127.0.0.1:7897"));
    }

    #[test]
    fn keeps_existing_scheme() {
        assert_eq!(
            normalize_server("socks5://127.0.0.1:1080").as_deref(),
            Some("socks5://127.0.0.1:1080")
        );
    }

    #[test]
    fn ie_semicolon_form() {
        assert_eq!(
            normalize_server("http=127.0.0.1:7890;https=127.0.0.1:7890;no_proxy=localhost")
                .as_deref(),
            Some("http://127.0.0.1:7890")
        );
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(normalize_server("   "), None);
    }

    #[test]
    fn detect_does_not_panic() {
        let p = detect();
        println!("detected: {:?} / {}", p.url, p.desc);
    }
}
