use std::collections::HashSet;

use url::Url;

#[derive(Debug, Clone)]
pub struct JsPolicy {
    default_enabled: bool,
    allow_domains: HashSet<String>,
}

impl JsPolicy {
    pub fn new(default_enabled: bool, allow_domains: Vec<String>) -> Self {
        let allow_domains = allow_domains
            .into_iter()
            .map(|d| d.trim().to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect();

        Self {
            default_enabled,
            allow_domains,
        }
    }

    pub fn is_enabled_for(&self, url: &Url) -> bool {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        self.default_enabled || self.allow_domains.contains(&host)
    }

    pub fn default_enabled(&self) -> bool {
        self.default_enabled
    }

    pub fn set_default_enabled(&mut self, enabled: bool) {
        self.default_enabled = enabled;
    }

    pub fn toggle_default(&mut self) {
        self.default_enabled = !self.default_enabled;
    }
}

#[derive(Debug, Clone)]
pub struct JsExecutionReport {
    pub enabled: bool,
    pub script_count: usize,
    pub note: String,
}

pub fn evaluate_document_scripts(enabled: bool, script_count: usize) -> JsExecutionReport {
    if enabled {
        JsExecutionReport {
            enabled,
            script_count,
            note: "JavaScript runtime placeholder active (DOM bridge is minimal in MVP)."
                .to_string(),
        }
    } else {
        JsExecutionReport {
            enabled,
            script_count,
            note: "JavaScript blocked by policy.".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::JsPolicy;
    use url::Url;

    #[test]
    fn allows_domain_when_listed() {
        let policy = JsPolicy::new(false, vec!["example.com".to_string()]);
        let url = Url::parse("https://example.com").expect("url");
        assert!(policy.is_enabled_for(&url));
    }

    #[test]
    fn blocks_domain_when_not_listed_and_default_off() {
        let policy = JsPolicy::new(false, vec!["example.com".to_string()]);
        let url = Url::parse("https://www.rust-lang.org").expect("url");
        assert!(!policy.is_enabled_for(&url));
    }

    #[test]
    fn toggles_default_policy() {
        let mut policy = JsPolicy::new(false, vec![]);
        assert!(!policy.default_enabled());
        policy.toggle_default();
        assert!(policy.default_enabled());
        policy.set_default_enabled(false);
        assert!(!policy.default_enabled());
    }
}
