//! AI 端点的地址策略（P0-3）：scheme 白名单、默认 https、明文只给本机与私网、
//! 公网主机只走 443、重定向只跟随同源且逐跳复核。
//!
//! 为什么不能「用户填什么就用什么」：这个地址来自设置文件，而设置文件可能被同步盘、
//! 备份还原或手改动过；`http://api.example.com` 会把 API Key 明文交给整条链路。
//! 本机与私网放行 http 是因为默认接入就是 Ollama（`127.0.0.1:11434` 没有 TLS），
//! 局域网自建服务同一处境 —— 这条线画在「跨出本机与局域网」上。
//!
//! 重定向只跟**同源**跳：跨源会把 `Authorization` 交给另一个主机，而我们没有任何理由
//! 替用户做这个决定（要换主机就自己把地址改掉）。同源跳里再按同一套策略复核一次。

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};

/// 重定向最多跟几跳
pub const MAX_REDIRECTS: usize = 3;

/// 已校验的端点
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// 规范化后的 base：补 scheme、去尾斜杠、没有版本段时补 `/v1`
    pub base: String,
    pub host: String,
    pub port: u16,
    pub https: bool,
    /// 回环 / 私网 / 链路本地 —— 唯一允许明文 http 的范围
    pub local: bool,
}

impl Target {
    /// 只用于日志：**不带路径**。反代和自建网关常把令牌挂在路径里，
    /// 而日志是会被贴出来求助的东西。
    pub fn origin(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        format!("{scheme}://{}:{}", self.host, self.port)
    }
}

/// 校验并规范化一个 base URL。返回的 `Target.base` 就是真正拼请求用的那一段。
pub fn check(raw: &str) -> Result<Target, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        // 与历史默认一致：本机 Ollama
        return finish("http://127.0.0.1:11434".to_string());
    }
    // 写了 scheme 就照写，别去「补」：`ftp://a` 补成 `https://ftp://a` 会被解析成
    // 「主机叫 ftp、路径是 //a」，一个明摆着的非 http 端点就混进来了。
    if let Some((scheme, _)) = trimmed.split_once("://") {
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return Err(format!("只接受 http/https 端点，收到 {scheme}://"));
        }
        return finish(trimmed.to_string());
    }
    finish(ensure_scheme(trimmed))
}

fn finish(with_scheme: String) -> Result<Target, String> {
    let url = reqwest::Url::parse(&with_scheme).map_err(|e| format!("地址无法解析：{e}"))?;
    let scheme = url.scheme().to_string();
    if scheme != "http" && scheme != "https" {
        return Err(format!("只接受 http/https 端点，收到 {scheme}://"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("地址里不要带用户名或密码：凭据只从 API Key 那一栏来".into());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "地址缺少主机名".to_string())?
        .to_ascii_lowercase();
    let https = scheme == "https";
    let local = is_local_host(&host);
    if !https && !local {
        return Err(format!(
            "公网主机 {host} 必须用 https：明文会把 API Key 交给链路上的任何一跳"
        ));
    }
    let port = url.port_or_known_default().ok_or_else(|| "地址端口无法确定".to_string())?;
    if !local && port != 443 {
        return Err(format!(
            "公网主机只允许 443 端口，收到 {port}：自建服务请用 443 或在其前面加一层反向代理"
        ));
    }
    let mut base = with_scheme.trim_end_matches('/').to_string();
    if !has_version_segment(&base) {
        base.push_str("/v1");
    }
    Ok(Target { base, host, port, https, local })
}

/// 没写 scheme 时补一个：本机/私网补 `http`，其余补 `https`。
///
/// 旧的实现一律补 `http` —— 用户打 `api.deepseek.com` 就打出一个明文端点，
/// 而那一栏里同时还放着 API Key。
fn ensure_scheme(raw: &str) -> String {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return raw.to_string();
    }
    let authority = raw.split(['/', '?', '#']).next().unwrap_or("");
    let host = host_of_authority(authority);
    let scheme = if is_local_host(host) { "http" } else { "https" };
    format!("{scheme}://{raw}")
}

/// 从 `user@host:port` 里取出主机（IPv6 字面量带方括号）
fn host_of_authority(authority: &str) -> &str {
    let a = authority.rsplit('@').next().unwrap_or(authority);
    if a.starts_with('[') {
        return a.split(']').next().unwrap_or(a);
    }
    match a.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => a,
    }
}

/// 回环 / 私网 / 链路本地。域名只认 `localhost` 系（`foo.localhost` 也解析到本机）；
/// 其它域名一律当公网 —— 不在这里做 DNS，也就不会被 DNS 重绑定绕过。
pub fn is_local_host(host: &str) -> bool {
    let h = host.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") {
        return true;
    }
    match h.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        Ok(IpAddr::V6(v6)) => {
            let head = v6.segments()[0];
            // fc00::/7 唯一本地地址、fe80::/10 链路本地
            v6.is_loopback() || v6.is_unspecified() || (head & 0xfe00) == 0xfc00 || (head & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// 等价 TS 正则 `/\/v\d+(\/|$)/`：末段或任一段为 v+数字
fn has_version_segment(u: &str) -> bool {
    u.split('/').any(|seg| {
        seg.strip_prefix('v')
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    })
}

/// 一次重定向该不该跟（纯函数，方便钉死规则）。
///
/// `previous` 是上一跳的完整 URL，`hops` 是已经跟过几跳。规则三条：
/// 不比 `MAX_REDIRECTS` 多、必须同源、目标本身还要过 [`check`]。
pub fn redirect_verdict(previous: Option<&str>, next: &str, hops: usize) -> Result<(), String> {
    if hops > MAX_REDIRECTS {
        return Err(format!("重定向超过 {MAX_REDIRECTS} 跳"));
    }
    let next_url = reqwest::Url::parse(next).map_err(|e| format!("重定向目标无法解析：{e}"))?;
    let Some(prev) = previous else {
        // 没有来路就没有同源可谈（正常流程里不该出现）
        return Ok(());
    };
    let prev_url = reqwest::Url::parse(prev).map_err(|e| format!("上一跳地址无法解析：{e}"))?;
    let same_origin = prev_url.scheme() == next_url.scheme()
        && prev_url.host_str() == next_url.host_str()
        && prev_url.port_or_known_default() == next_url.port_or_known_default();
    if !same_origin {
        return Err(format!(
            "拒绝跨源重定向：{} → {}（要把请求发到别的主机，请直接改接口地址）",
            origin_of(&prev_url),
            origin_of(&next_url)
        ));
    }
    check(next).map(|_| ())
}

fn origin_of(u: &reqwest::Url) -> String {
    format!("{}://{}:{}", u.scheme(), u.host_str().unwrap_or(""), u.port_or_known_default().unwrap_or(0))
}

/// 端点身份只记一次：日志里是「哪台主机 + Key 有没有、多长」，**永远不写 Key 本体**。
/// 记一次的粒度按 origin 算 —— 每块请求都写一遍会把日志冲干净，也拖慢任务线程。
pub fn note_endpoint(target: &Target, api_key: &str) {
    static LOGGED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let origin = target.origin();
    let seen = LOGGED.get_or_init(|| Mutex::new(HashSet::new()));
    let first = match seen.lock() {
        Ok(mut set) => set.insert(origin.clone()),
        Err(_) => false,
    };
    if !first {
        return;
    }
    let key = crate::ai::redact(api_key);
    crate::paths::append_log(&format!("[ai] 端点 {origin} · API Key {key}"));
}

/// 给 `reqwest::Client` 用的重定向策略：逐跳走 [`redirect_verdict`]，拒了就带上原因。
pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let prev = attempt.previous().last().map(|u| u.as_str().to_string());
        match redirect_verdict(prev.as_deref(), attempt.url().as_str(), attempt.previous().len()) {
            Ok(()) => attempt.follow(),
            Err(e) => attempt.error(e),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(raw: &str) -> Target {
        check(raw).unwrap_or_else(|e| panic!("{raw} 应当通过：{e}"))
    }

    fn rejected(raw: &str) -> String {
        check(raw).err().unwrap_or_else(|| panic!("{raw} 应当被拒"))
    }

    #[test]
    fn a_missing_scheme_defaults_to_https_on_the_internet() {
        assert_eq!(ok("api.deepseek.com").base, "https://api.deepseek.com/v1");
        assert!(ok("api.deepseek.com").https);
    }

    #[test]
    fn local_models_keep_plain_http() {
        for raw in ["127.0.0.1:11434", "localhost:8080", "192.168.1.9:1234", "[::1]:11434"] {
            let t = ok(raw);
            assert!(!t.https, "{raw} 是本机/私网，明文放行");
            assert!(t.local, "{raw} 应判为 local");
        }
        // 空地址 = 历史默认（本机 Ollama）
        assert_eq!(ok("  ").base, "http://127.0.0.1:11434/v1");
    }

    #[test]
    fn public_hosts_must_be_https() {
        let e = rejected("http://api.deepseek.com/v1");
        assert!(e.contains("必须用 https"), "{e}");
    }

    #[test]
    fn public_hosts_may_only_use_port_443() {
        assert_eq!(ok("https://api.deepseek.com:443/v1").port, 443);
        let e = rejected("https://api.deepseek.com:8443/v1");
        assert!(e.contains("443 端口"), "{e}");
        // 私网/本机端口不限：自建服务的端口是用户自己定的
        assert_eq!(ok("http://192.168.1.5:9999/v1").port, 9999);
    }

    #[test]
    fn non_http_schemes_and_embedded_credentials_are_refused() {
        assert!(rejected("ftp://example.com").contains("只接受 http/https"));
        assert!(rejected("file:///etc/passwd").contains("只接受 http/https"));
        assert!(rejected("https://user:pass@example.com/v1").contains("不要带用户名或密码"));
    }

    #[test]
    fn private_ranges_are_recognized_without_dns() {
        for h in ["10.0.0.1", "172.16.5.4", "192.168.0.10", "169.254.1.1", "127.1.2.3", "::1", "fd00::1", "fe80::1"] {
            assert!(is_local_host(h), "{h} 是本机/私网");
        }
        for h in ["api.deepseek.com", "example.com", "8.8.8.8", "172.32.0.1", "100.64.0.1"] {
            assert!(!is_local_host(h), "{h} 应当按公网对待");
        }
    }

    #[test]
    fn version_segment_detection_is_unchanged() {
        assert_eq!(ok("https://api.deepseek.com/v1").base, "https://api.deepseek.com/v1");
        assert_eq!(ok("https://api.deepseek.com/v1/").base, "https://api.deepseek.com/v1");
        assert_eq!(ok("https://api.deepseek.com/v2beta").base, "https://api.deepseek.com/v2beta/v1");
    }

    #[test]
    fn redirects_are_same_origin_and_bounded() {
        assert!(redirect_verdict(Some("https://api.x.com/v1/a"), "https://api.x.com/v1/b", 1).is_ok());
        let cross = redirect_verdict(Some("https://api.x.com/v1/a"), "https://evil.example/v1", 1).unwrap_err();
        assert!(cross.contains("拒绝跨源重定向"), "{cross}");
        // 换端口也算跨源（同主机不同服务）
        assert!(redirect_verdict(Some("https://api.x.com/a"), "https://api.x.com:8443/a", 1).is_err());
        let many = redirect_verdict(Some("https://api.x.com/a"), "https://api.x.com/b", MAX_REDIRECTS + 1).unwrap_err();
        assert!(many.contains("重定向超过"), "{many}");
    }

    #[test]
    fn a_redirect_to_an_unacceptable_target_is_refused_even_when_same_origin() {
        // 同源但目标不合规（明文 + 非本机）——正常流程走不到，规则本身要成立
        let e = redirect_verdict(Some("http://api.x.com/v1"), "http://api.x.com/v1/b", 1).unwrap_err();
        assert!(e.contains("必须用 https"), "{e}");
    }

    #[test]
    fn the_logged_origin_never_carries_the_path() {
        // 反代常把令牌挂在路径里，日志只该出现 scheme://host:port
        let t = ok("https://gateway.example.com/v1/sk-live-abcdef");
        assert_eq!(t.origin(), "https://gateway.example.com:443");
        assert!(!t.origin().contains("sk-live"));
    }
}
