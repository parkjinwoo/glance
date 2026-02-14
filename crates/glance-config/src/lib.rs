use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub startup: StartupConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    pub cookies: CookieConfig,
    #[serde(default)]
    pub js: JsConfig,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupConfig {
    pub home: String,
    pub default_js: JsDefault,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub user_agent: String,
    pub timeout_ms: u64,
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
    pub mode: CookieMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JsConfig {
    #[serde(default)]
    pub allow_domains: Vec<String>,
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
    pub allowed_players: Vec<String>,
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

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum JsDefault {
    Off,
    Ask,
    On,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CookieMode {
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
pub enum ExternalPlayerMode {
    #[default]
    Sync,
    Spawn,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            startup: StartupConfig {
                home: "".to_string(),
                default_js: JsDefault::On,
            },
            network: NetworkConfig {
                user_agent: "glance/0.1".to_string(),
                timeout_ms: 15_000,
                max_redirects: 10,
                max_body_bytes: default_max_body_bytes(),
                max_image_bytes: default_max_image_bytes(),
                allow_schemes: default_allow_schemes(),
            },
            cookies: CookieConfig {
                mode: CookieMode::Session,
            },
            js: JsConfig {
                allow_domains: Vec::new(),
            },
            media: MediaConfig::default(),
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

        Ok(cfg)
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

fn default_max_image_bytes() -> usize {
    8 * 1024 * 1024
}

fn default_allow_schemes() -> Vec<String> {
    vec!["http".to_string(), "https".to_string(), "about".to_string()]
}

fn default_external_player() -> String {
    "mpv".to_string()
}

fn default_sanitize_control_chars() -> bool {
    true
}

fn default_allow_private_ip() -> bool {
    true
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            user_agent: "glance/0.1".to_string(),
            timeout_ms: 15_000,
            max_redirects: 10,
            max_body_bytes: default_max_body_bytes(),
            max_image_bytes: default_max_image_bytes(),
            allow_schemes: default_allow_schemes(),
        }
    }
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            image_mode: ImageMode::Auto,
            external_player: default_external_player(),
            external_player_mode: ExternalPlayerMode::Sync,
            allowed_players: Vec::new(),
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
