use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use encoding_rs::{Encoding, UTF_8};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue, LOCATION};
use reqwest::{Client, StatusCode, redirect};
use reqwest_cookie_store::{CookieStore, CookieStoreMutex};
use url::{Host, Url, form_urlencoded};

#[derive(Debug, Clone)]
pub struct FetchResult {
    pub requested_url: Url,
    pub final_url: Url,
    pub status: u16,
    pub content_type: Option<String>,
    pub location: Option<String>,
    pub redirect_chain: Vec<RedirectHop>,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct RedirectHop {
    pub from: Url,
    pub to: Url,
    pub status: u16,
}

#[derive(Debug, Clone)]
pub struct NetworkAccessPolicy {
    allowed_schemes: Vec<String>,
    allow_private_ip: bool,
}

impl NetworkAccessPolicy {
    pub fn new(allowed_schemes: Vec<String>, allow_private_ip: bool) -> Result<Self> {
        let mut allowed_schemes = allowed_schemes
            .into_iter()
            .map(|scheme| scheme.trim().to_ascii_lowercase())
            .filter(|scheme| !scheme.is_empty())
            .collect::<Vec<_>>();
        allowed_schemes.sort();
        allowed_schemes.dedup();
        if allowed_schemes.is_empty() {
            bail!("network scheme allowlist cannot be empty");
        }
        Ok(Self {
            allowed_schemes,
            allow_private_ip,
        })
    }

    pub fn validate(&self, url: &Url) -> Result<()> {
        let scheme = url.scheme().to_ascii_lowercase();
        if !self
            .allowed_schemes
            .iter()
            .any(|allowed| allowed == &scheme)
        {
            bail!("scheme '{}' is not allowed by config", url.scheme());
        }
        if !self.allow_private_ip && is_non_public_host(url) {
            bail!("private, loopback, or otherwise non-public target is blocked by config");
        }
        Ok(())
    }

    async fn validate_resolved_host(&self, url: &Url) -> Result<()> {
        if self.allow_private_ip || !matches!(url.host(), Some(Host::Domain(_))) {
            return Ok(());
        }
        let host = url.host_str().context("network URL has no host")?;
        let port = url
            .port_or_known_default()
            .context("network URL has no known port")?;
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .with_context(|| format!("failed to resolve host {host}"))?
            .collect::<Vec<_>>();
        if addresses.is_empty() {
            bail!("host {host} resolved to no addresses");
        }
        if addresses
            .iter()
            .any(|address| is_non_public_ip(address.ip()))
        {
            bail!("host {host} resolves to a non-public address blocked by config");
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct NetClient {
    client: Client,
    max_redirects: usize,
    cookie_store: Arc<CookieStoreMutex>,
    cookie_path: Option<PathBuf>,
}

impl NetClient {
    pub fn new(
        user_agent: &str,
        timeout_ms: u64,
        max_redirects: usize,
        cookie_path: Option<PathBuf>,
    ) -> Result<Self> {
        let cookie_store = Arc::new(CookieStoreMutex::new(load_cookie_store(
            cookie_path.as_deref(),
        )?));

        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.5"),
        );

        let client = Client::builder()
            .user_agent(user_agent)
            .default_headers(headers)
            .timeout(Duration::from_millis(timeout_ms))
            .redirect(redirect::Policy::none())
            .cookie_provider(Arc::clone(&cookie_store))
            .build()
            .context("failed to build reqwest client")?;

        Ok(Self {
            client,
            max_redirects,
            cookie_store,
            cookie_path,
        })
    }

    pub fn save_cookies(&self) -> Result<()> {
        let Some(path) = self.cookie_path.as_deref() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create cookie directory {}", parent.display())
            })?;
        }

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = tempfile::Builder::new()
            .prefix(".glance-cookies-")
            .tempfile_in(parent)
            .with_context(|| format!("failed to create cookie store in {}", parent.display()))?;
        #[cfg(unix)]
        temp.as_file()
            .set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .context("failed to restrict temporary cookie-store permissions")?;
        {
            let mut writer = BufWriter::new(temp.as_file_mut());
            let store = self
                .cookie_store
                .lock()
                .map_err(|_| anyhow!("cookie store lock is poisoned"))?;
            cookie_store::serde::json::save(&store, &mut writer)
                .map_err(|err| anyhow!("failed to serialize cookie store: {err}"))?;
            drop(store);
            writer.flush().context("failed to flush cookie store")?;
            writer
                .get_ref()
                .sync_all()
                .context("failed to sync cookie store")?;
        }

        // Unix rename replaces atomically. Windows does not replace an existing file,
        // so remove only the exact cookie destination before moving the completed file.
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path)
                .with_context(|| format!("failed to remove old cookie store {}", path.display()))?;
        }
        temp.persist(path).map_err(|error| {
            anyhow!(
                "failed to replace cookie store {} with {}: {}",
                path.display(),
                error.file.path().display(),
                error.error
            )
        })?;
        Ok(())
    }

    pub async fn fetch(&self, url: &Url) -> Result<FetchResult> {
        self.fetch_limited(url, usize::MAX).await
    }

    pub async fn fetch_limited(&self, url: &Url, max_body_bytes: usize) -> Result<FetchResult> {
        let policy = NetworkAccessPolicy::new(vec!["http".to_string(), "https".to_string()], true)?;
        self.fetch_limited_with_policy(url, max_body_bytes, &policy)
            .await
    }

    pub async fn fetch_limited_with_policy(
        &self,
        url: &Url,
        max_body_bytes: usize,
        policy: &NetworkAccessPolicy,
    ) -> Result<FetchResult> {
        let requested_url = url.clone();
        let mut current_url = requested_url.clone();
        let mut redirect_chain = Vec::new();

        loop {
            policy.validate(&current_url)?;
            policy.validate_resolved_host(&current_url).await?;
            let res = self
                .client
                .get(current_url.clone())
                .send()
                .await
                .with_context(|| format!("request failed for {current_url}"))?;
            let status = res.status();
            let location = header_string(res.headers(), LOCATION);

            if is_redirect_follow_status(status) {
                if redirect_chain.len() >= self.max_redirects {
                    bail!("redirect limit exceeded ({})", self.max_redirects);
                }
                let location = location.context("redirect response missing location header")?;
                let target =
                    resolve_redirect_target(&current_url, &location).with_context(|| {
                        format!(
                            "failed to resolve redirect location '{location}' from {current_url}"
                        )
                    })?;
                policy
                    .validate(&target)
                    .with_context(|| format!("redirect target blocked: {target}"))?;
                redirect_chain.push(RedirectHop {
                    from: current_url.clone(),
                    to: target.clone(),
                    status: status.as_u16(),
                });
                current_url = target;
                continue;
            }

            let final_url = res.url().clone();
            policy.validate(&final_url)?;
            let content_type = header_string(res.headers(), CONTENT_TYPE);
            let body_bytes = read_response_bytes_limited(res, max_body_bytes)
                .await
                .context("failed to read response body")?;
            let body = decode_body(&body_bytes, content_type.as_deref());

            return Ok(FetchResult {
                requested_url,
                final_url,
                status: status.as_u16(),
                content_type,
                location,
                redirect_chain,
                body,
            });
        }
    }

    pub async fn fetch_bytes(&self, url: &Url) -> Result<Vec<u8>> {
        self.fetch_bytes_limited(url, usize::MAX).await
    }

    pub async fn fetch_bytes_limited(&self, url: &Url, max_bytes: usize) -> Result<Vec<u8>> {
        let policy = NetworkAccessPolicy::new(vec!["http".to_string(), "https".to_string()], true)?;
        self.fetch_bytes_limited_with_policy(url, max_bytes, &policy)
            .await
    }

    pub async fn fetch_bytes_limited_with_policy(
        &self,
        url: &Url,
        max_bytes: usize,
        policy: &NetworkAccessPolicy,
    ) -> Result<Vec<u8>> {
        let mut current_url = url.clone();
        let mut redirects = 0usize;

        loop {
            policy.validate(&current_url)?;
            policy.validate_resolved_host(&current_url).await?;
            let res = self
                .client
                .get(current_url.clone())
                .send()
                .await
                .with_context(|| format!("request failed for {current_url}"))?;
            let status = res.status();

            if is_redirect_follow_status(status) {
                if redirects >= self.max_redirects {
                    bail!("redirect limit exceeded ({})", self.max_redirects);
                }
                let location = res
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .context("redirect response missing location header")?;
                let target =
                    resolve_redirect_target(&current_url, location).with_context(|| {
                        format!(
                            "failed to resolve redirect location '{location}' from {current_url}"
                        )
                    })?;
                policy
                    .validate(&target)
                    .with_context(|| format!("redirect target blocked: {target}"))?;
                current_url = target;
                redirects = redirects.saturating_add(1);
                continue;
            }

            policy.validate(res.url())?;
            return read_response_bytes_limited(res, max_bytes)
                .await
                .context("failed to read response bytes");
        }
    }
}

fn load_cookie_store(path: Option<&Path>) -> Result<CookieStore> {
    let Some(path) = path else {
        return Ok(CookieStore::new());
    };
    if !path.exists() {
        return Ok(CookieStore::new());
    }
    let file = File::open(path)
        .with_context(|| format!("failed to open cookie store {}", path.display()))?;
    cookie_store::serde::json::load(BufReader::new(file))
        .map_err(|err| anyhow!("failed to parse cookie store {}: {err}", path.display()))
}

fn header_string(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

async fn read_response_bytes_limited(
    mut res: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    if let Some(length) = res.content_length()
        && length > max_bytes as u64
    {
        bail!("response body exceeds configured limit ({max_bytes} bytes)");
    }

    let mut out = Vec::new();
    while let Some(chunk) = res.chunk().await.context("failed to read response chunk")? {
        if out.len().saturating_add(chunk.len()) > max_bytes {
            bail!("response body exceeds configured limit ({max_bytes} bytes)");
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn decode_body(bytes: &[u8], content_type: Option<&str>) -> String {
    let label = content_type
        .and_then(charset_from_content_type)
        .or_else(|| sniff_html_charset(bytes));
    let encoding = label
        .as_deref()
        .and_then(|label| Encoding::for_label(label.as_bytes()))
        .unwrap_or(UTF_8);
    let (decoded, _, _) = encoding.decode(bytes);
    decoded.into_owned()
}

fn charset_from_content_type(content_type: &str) -> Option<String> {
    content_type.split(';').skip(1).find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("charset") {
            return None;
        }
        let value = value.trim().trim_matches(['\'', '"']);
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn sniff_html_charset(bytes: &[u8]) -> Option<String> {
    let prefix = &bytes[..bytes.len().min(2048)];
    let ascii = String::from_utf8_lossy(prefix).to_ascii_lowercase();
    let marker = "charset=";
    let start = ascii.find(marker)?.saturating_add(marker.len());
    let rest = ascii
        .get(start..)?
        .trim_start_matches([' ', '\t', '\'', '"']);
    let end = rest
        .find(|ch: char| ch.is_ascii_whitespace() || matches!(ch, '\'' | '"' | '>' | ';'))
        .unwrap_or(rest.len());
    let label = rest.get(..end)?.trim();
    (!label.is_empty()).then(|| label.to_string())
}

fn is_redirect_follow_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn resolve_redirect_target(base: &Url, location: &str) -> Result<Url> {
    if let Ok(absolute) = Url::parse(location) {
        return Ok(absolute);
    }
    base.join(location)
        .with_context(|| format!("invalid redirect target: {location}"))
}

pub fn resolve_user_input(input: &str, search_template: &str) -> Result<Url> {
    let input = input.trim();
    if input.is_empty() {
        bail!("address or search query is empty");
    }
    if looks_like_url(input) {
        return normalize_url(input);
    }
    if !search_template.contains("{query}") {
        bail!("search template must contain {{query}}");
    }
    let encoded = form_urlencoded::byte_serialize(input.as_bytes()).collect::<String>();
    let search_url = search_template.replace("{query}", &encoded);
    Url::parse(&search_url).with_context(|| format!("invalid search template result: {search_url}"))
}

fn looks_like_url(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    if lower.starts_with("about:")
        || lower.starts_with("http:")
        || lower.starts_with("https:")
        || input.contains("://")
    {
        return true;
    }
    if input.chars().any(char::is_whitespace) {
        return false;
    }

    let authority = input.split(['/', '?', '#']).next().unwrap_or(input);
    let host = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority)
        .split(':')
        .next()
        .unwrap_or(authority);
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok()
        || (host.contains('.') && !host.starts_with('.') && !host.ends_with('.'))
}

pub fn normalize_url(input: &str) -> Result<Url> {
    let input = input.trim();
    if input.is_empty() {
        bail!("failed to parse url: input is empty");
    }

    match Url::parse(input) {
        Ok(url) => Ok(url),
        Err(_) => {
            if let Some(fixed) = fix_missing_authority(input)
                && let Ok(url) = Url::parse(&fixed)
            {
                return Ok(url);
            }

            let with_scheme = format!("https://{input}");
            Url::parse(&with_scheme)
                .with_context(|| format!("failed to parse url: {input} (also tried {with_scheme})"))
        }
    }
}

fn fix_missing_authority(input: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();

    for scheme in ["http", "https"] {
        let prefix = format!("{scheme}:");
        if lower.starts_with(&prefix) {
            let remainder = input[prefix.len()..].trim_start_matches('/');
            if remainder.is_empty() {
                return None;
            }
            return Some(format!("{scheme}://{remainder}"));
        }
    }

    None
}

fn is_non_public_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => is_non_public_ip(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => is_non_public_ip(IpAddr::V6(ip)),
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

fn is_non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 192 && b == 168)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(ip) => {
            if let Some(ipv4) = ip.to_ipv4_mapped() {
                return is_non_public_ip(IpAddr::V4(ipv4));
            }
            let segments = ip.segments();
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::{IpAddr, Ipv6Addr, TcpListener};
    use std::thread;

    use super::{
        NetClient, NetworkAccessPolicy, StatusCode, decode_body, is_non_public_ip,
        is_redirect_follow_status, normalize_url, resolve_redirect_target, resolve_user_input,
    };
    use url::Url;

    #[test]
    fn parses_full_url() {
        let url = normalize_url("https://example.com/path").expect("url");
        assert_eq!(url.as_str(), "https://example.com/path");
    }

    #[test]
    fn adds_https_scheme() {
        let url = normalize_url("example.com").expect("url");
        assert_eq!(url.as_str(), "https://example.com/");
    }

    #[test]
    fn fixes_https_without_slashes() {
        let url = normalize_url("https:example.com").expect("url");
        assert_eq!(url.as_str(), "https://example.com/");
    }

    #[test]
    fn supports_about_blank() {
        let url = normalize_url("about:blank").expect("url");
        assert_eq!(url.as_str(), "about:blank");
    }

    #[test]
    fn rejects_empty_input() {
        let err = normalize_url("  ").expect_err("expected empty input error");
        assert!(err.to_string().contains("input is empty"));
    }

    #[test]
    fn resolves_relative_redirect_target() {
        let base = Url::parse("https://example.com/path/page").expect("url");
        let target = resolve_redirect_target(&base, "/next").expect("target");
        assert_eq!(target.as_str(), "https://example.com/next");
    }

    #[test]
    fn follows_expected_redirect_statuses() {
        assert!(is_redirect_follow_status(StatusCode::MOVED_PERMANENTLY));
        assert!(is_redirect_follow_status(StatusCode::FOUND));
        assert!(!is_redirect_follow_status(StatusCode::NOT_MODIFIED));
    }

    #[test]
    fn fixes_http_without_slashes() {
        let url = normalize_url("http:example.com").expect("url");
        assert_eq!(url.as_str(), "http://example.com/");
    }

    #[test]
    fn turns_plain_words_into_search_url() {
        let url = resolve_user_input("rust tui browser", "https://search.example/?q={query}")
            .expect("search url");
        assert_eq!(url.as_str(), "https://search.example/?q=rust+tui+browser");
    }

    #[test]
    fn preserves_domain_like_input() {
        let url = resolve_user_input("example.com/docs", "https://search/?q={query}").expect("url");
        assert_eq!(url.as_str(), "https://example.com/docs");
    }

    #[test]
    fn blocks_private_redirect_targets_when_requested() {
        let policy = NetworkAccessPolicy::new(vec!["http".to_string(), "https".to_string()], false)
            .expect("policy");
        let target = Url::parse("http://127.0.0.1/admin").expect("url");
        assert!(policy.validate(&target).is_err());
    }

    #[test]
    fn blocks_ipv4_mapped_private_ipv6_addresses() {
        let mapped_loopback = "::ffff:127.0.0.1"
            .parse::<Ipv6Addr>()
            .expect("mapped IPv6 address");
        assert!(is_non_public_ip(IpAddr::V6(mapped_loopback)));
    }

    #[test]
    fn decodes_legacy_charset_from_header() {
        let bytes = [0xC7, 0xD1, 0xB1, 0xDB];
        assert_eq!(
            decode_body(&bytes, Some("text/html; charset=euc-kr")),
            "한글"
        );
    }

    #[tokio::test]
    async fn rejects_redirect_to_disallowed_scheme() {
        let url = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: file:///etc/passwd\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let client = NetClient::new("glance-test", 2_000, 3, None).expect("client");
        let policy = NetworkAccessPolicy::new(vec!["http".to_string(), "https".to_string()], true)
            .expect("policy");

        let error = client
            .fetch_limited_with_policy(&url, 1024, &policy)
            .await
            .expect_err("redirect should be blocked");
        assert!(error.to_string().contains("redirect target blocked"));
    }

    #[tokio::test]
    async fn rejects_body_larger_than_limit() {
        let url = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
        );
        let client = NetClient::new("glance-test", 2_000, 3, None).expect("client");
        let policy = NetworkAccessPolicy::new(vec!["http".to_string()], true).expect("policy");

        let error = client
            .fetch_limited_with_policy(&url, 4, &policy)
            .await
            .expect_err("body should be too large");
        assert!(format!("{error:#}").contains("configured limit"));
    }

    #[cfg(unix)]
    #[test]
    fn persistent_cookie_store_is_private_and_leaves_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("cookies.json");
        let client = NetClient::new("glance-test", 2_000, 3, Some(path.clone())).expect("client");
        client.save_cookies().expect("save cookies");

        let mode = path
            .metadata()
            .expect("cookie metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let names = std::fs::read_dir(root.path())
            .expect("cookie directory")
            .map(|entry| entry.expect("entry").file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, vec![std::ffi::OsString::from("cookies.json")]);
    }

    fn serve_once(response: &'static str) -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("connection");
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request);
            stream.write_all(response.as_bytes()).expect("response");
        });
        Url::parse(&format!("http://{address}/")).expect("url")
    }
}
