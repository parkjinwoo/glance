use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::header::{CONTENT_TYPE, LOCATION};
use reqwest::{Client, StatusCode, redirect};
use url::Url;

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

#[derive(Clone)]
pub struct NetClient {
    client: Client,
    max_redirects: usize,
}

impl NetClient {
    pub fn new(user_agent: &str, timeout_ms: u64, max_redirects: usize) -> Result<Self> {
        let client = Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_millis(timeout_ms))
            .redirect(redirect::Policy::none())
            .build()
            .context("failed to build reqwest client")?;

        Ok(Self {
            client,
            max_redirects,
        })
    }

    pub async fn fetch(&self, url: &Url) -> Result<FetchResult> {
        self.fetch_limited(url, usize::MAX).await
    }

    pub async fn fetch_limited(&self, url: &Url, max_body_bytes: usize) -> Result<FetchResult> {
        let requested_url = url.clone();
        let mut current_url = requested_url.clone();
        let mut redirect_chain = Vec::new();

        loop {
            let res = self
                .client
                .get(current_url.clone())
                .send()
                .await
                .with_context(|| format!("request failed for {current_url}"))?;
            let status = res.status();
            let location = res
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(ToString::to_string);

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
                redirect_chain.push(RedirectHop {
                    from: current_url.clone(),
                    to: target.clone(),
                    status: status.as_u16(),
                });
                current_url = target;
                continue;
            }

            let final_url = res.url().clone();
            let content_type = res
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(ToString::to_string);
            let body_bytes = read_response_bytes_limited(res, max_body_bytes)
                .await
                .context("failed to read response body")?;
            let body = String::from_utf8_lossy(&body_bytes).into_owned();

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
        let mut current_url = url.clone();
        let mut redirects = 0usize;

        loop {
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
                    .and_then(|v| v.to_str().ok())
                    .context("redirect response missing location header")?;
                current_url =
                    resolve_redirect_target(&current_url, location).with_context(|| {
                        format!(
                            "failed to resolve redirect location '{location}' from {current_url}"
                        )
                    })?;
                redirects = redirects.saturating_add(1);
                continue;
            }

            let bytes = read_response_bytes_limited(res, max_bytes)
                .await
                .context("failed to read response bytes")?;
            return Ok(bytes);
        }
    }
}

async fn read_response_bytes_limited(
    mut res: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = res.chunk().await.context("failed to read response chunk")? {
        if out.len().saturating_add(chunk.len()) > max_bytes {
            bail!(
                "response body exceeds configured limit ({} bytes)",
                max_bytes
            );
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::{StatusCode, is_redirect_follow_status, normalize_url, resolve_redirect_target};
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
}
