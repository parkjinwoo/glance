use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub startup: StartupConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub cookies: CookieConfig,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub dynamic: DynamicConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StartupConfig {
    #[serde(default)]
    pub home: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_max_redirects")]
    pub max_redirects: usize,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    #[serde(default = "default_max_image_bytes")]
    pub max_image_bytes: usize,
    #[serde(default = "default_allow_schemes")]
    pub allow_schemes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieConfig {
    #[serde(default)]
    pub mode: CookieMode,
    #[serde(default)]
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    #[serde(default = "default_search_template")]
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    #[serde(default)]
    pub image_mode: ImageMode,
    #[serde(default = "default_external_player")]
    pub external_player: String,
    #[serde(default)]
    pub external_player_mode: ExternalPlayerMode,
    #[serde(default)]
    pub external_player_args: Vec<String>,
    #[serde(default)]
    pub allowed_players: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicConfig {
    #[serde(default)]
    pub mode: DynamicMode,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default = "default_dynamic_wait_ms")]
    pub wait_ms: u64,
    #[serde(default = "default_dynamic_timeout_ms")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_sanitize_control_chars")]
    pub sanitize_control_chars: bool,
    #[serde(default = "default_allow_private_ip")]
    pub allow_private_ip: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UiConfig {
    #[serde(default)]
    pub icon_mode: UiIconMode,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum UiIconMode {
    #[default]
    Auto,
    Ascii,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CookieMode {
    #[default]
    Session,
    Persistent,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ImageMode {
    #[default]
    Auto,
    Text,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DynamicMode {
    Off,
    Manual,
    #[default]
    Auto,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExternalPlayerMode {
    Sync,
    #[default]
    Spawn,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            startup: StartupConfig {
                home: "".to_string(),
            },
            network: NetworkConfig {
                user_agent: default_user_agent(),
                timeout_ms: default_timeout_ms(),
                max_redirects: default_max_redirects(),
                max_body_bytes: default_max_body_bytes(),
                max_image_bytes: default_max_image_bytes(),
                allow_schemes: default_allow_schemes(),
            },
            cookies: CookieConfig {
                mode: CookieMode::Session,
                path: None,
            },
            search: SearchConfig::default(),
            media: MediaConfig::default(),
            dynamic: DynamicConfig::default(),
            security: SecurityConfig::default(),
            ui: UiConfig {
                icon_mode: UiIconMode::Auto,
            },
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let cfg_path = path.map(PathBuf::from).unwrap_or_else(default_config_path);

        if !cfg_path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&cfg_path)
            .with_context(|| format!("failed to read config at {}", cfg_path.display()))?;

        let cfg = toml::from_str::<Self>(&content)
            .with_context(|| format!("failed to parse TOML config at {}", cfg_path.display()))?;

        cfg.validate()
            .with_context(|| format!("invalid config at {}", cfg_path.display()))?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.network.timeout_ms > 0,
            "network.timeout_ms must be positive"
        );
        anyhow::ensure!(
            self.network.max_body_bytes > 0,
            "network.max_body_bytes must be positive"
        );
        anyhow::ensure!(
            self.network.max_image_bytes > 0,
            "network.max_image_bytes must be positive"
        );
        anyhow::ensure!(self.dynamic.wait_ms > 0, "dynamic.wait_ms must be positive");
        anyhow::ensure!(
            self.dynamic.timeout_ms >= self.dynamic.wait_ms,
            "dynamic.timeout_ms must be greater than or equal to dynamic.wait_ms"
        );
        anyhow::ensure!(
            self.search.template.contains("{query}"),
            "search.template must contain {{query}}"
        );
        Ok(())
    }
}

fn default_config_path() -> PathBuf {
    let mut base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.push("glance");
    base.push("config.toml");
    base
}

fn default_max_body_bytes() -> usize {
    4 * 1024 * 1024
}

fn default_user_agent() -> String {
    format!("glance/{}", env!("CARGO_PKG_VERSION"))
}

fn default_timeout_ms() -> u64 {
    15_000
}

fn default_max_redirects() -> usize {
    10
}

fn default_max_image_bytes() -> usize {
    8 * 1024 * 1024
}

fn default_allow_schemes() -> Vec<String> {
    vec!["http".to_string(), "https".to_string(), "about".to_string()]
}

fn default_external_player() -> String {
    "mpv".to_string()
}

fn default_search_template() -> String {
    "https://html.duckduckgo.com/html/?q={query}".to_string()
}

fn default_sanitize_control_chars() -> bool {
    true
}

fn default_allow_private_ip() -> bool {
    true
}

fn default_dynamic_wait_ms() -> u64 {
    10_000
}

fn default_dynamic_timeout_ms() -> u64 {
    30_000
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            user_agent: default_user_agent(),
            timeout_ms: default_timeout_ms(),
            max_redirects: default_max_redirects(),
            max_body_bytes: default_max_body_bytes(),
            max_image_bytes: default_max_image_bytes(),
            allow_schemes: default_allow_schemes(),
        }
    }
}

impl Default for CookieConfig {
    fn default() -> Self {
        Self {
            mode: CookieMode::Session,
            path: None,
        }
    }
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            template: default_search_template(),
        }
    }
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            image_mode: ImageMode::Auto,
            external_player: default_external_player(),
            external_player_mode: ExternalPlayerMode::Spawn,
            external_player_args: Vec::new(),
            allowed_players: Vec::new(),
        }
    }
}

impl Default for DynamicConfig {
    fn default() -> Self {
        Self {
            mode: DynamicMode::Auto,
            executable: None,
            wait_ms: default_dynamic_wait_ms(),
            timeout_ms: default_dynamic_timeout_ms(),
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            sanitize_control_chars: default_sanitize_control_chars(),
            allow_private_ip: default_allow_private_ip(),
        }
    }
}

pub fn default_cookie_store_path() -> PathBuf {
    let mut base = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.push("glance");
    base.push("cookies.json");
    base
}

pub fn default_browser_profile_path() -> PathBuf {
    let mut base = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.push("glance");
    base.push("browser-profile");
    base
}

#[cfg(test)]
mod tests {
    use super::{Config, DynamicMode};

    #[test]
    fn partial_config_uses_section_defaults() {
        let config: Config = toml::from_str(
            r#"
            [network]
            timeout_ms = 5000
            user_agent = "test"
            max_redirects = 4
            "#,
        )
        .expect("partial config");

        assert_eq!(config.network.timeout_ms, 5000);
        assert!(config.startup.home.is_empty());
        assert!(config.search.template.contains("{query}"));
        assert!(config.media.external_player_args.is_empty());
        assert_eq!(config.dynamic.mode, DynamicMode::Auto);
        assert_eq!(config.dynamic.wait_ms, 10_000);
    }
}
