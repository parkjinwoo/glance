use anyhow::Result;
use url::Url;

use glance_config::{
    Config, DynamicMode as ConfigDynamicMode, ExternalPlayerMode as ConfigExternalPlayerMode,
    ImageMode as ConfigImageMode,
};
use glance_media::ExternalPlayerLaunchMode;
use glance_net::NetworkAccessPolicy;

#[derive(Debug, Clone)]
pub(crate) struct NetworkPolicy {
    pub(crate) access: NetworkAccessPolicy,
    pub(crate) max_body_bytes: usize,
    pub(crate) max_image_bytes: usize,
    pub(crate) sanitize_control_chars: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct MediaPolicy {
    pub(crate) player: String,
    pub(crate) mode: ExternalPlayerLaunchMode,
    pub(crate) player_args: Vec<String>,
    pub(crate) allowed_players: Vec<String>,
    pub(crate) images_enabled: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum DynamicMode {
    Off,
    Manual,
    Auto,
}

#[derive(Debug, Clone)]
pub(crate) struct DynamicPolicy {
    pub(crate) mode: DynamicMode,
    pub(crate) executable: Option<String>,
    pub(crate) wait_ms: u64,
    pub(crate) timeout_ms: u64,
    pub(crate) security_compatible: bool,
}

pub(crate) fn normalize_scheme_list(input: &[String]) -> Vec<String> {
    input
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

pub(crate) fn is_about_blank(url: &Url) -> bool {
    url.scheme().eq_ignore_ascii_case("about") && url.path().eq_ignore_ascii_case("blank")
}

pub(crate) fn validate_url_for_network(url: &Url, access: &NetworkAccessPolicy) -> Result<()> {
    access.validate(url)
}

pub(crate) fn map_player_mode(mode: ConfigExternalPlayerMode) -> ExternalPlayerLaunchMode {
    match mode {
        ConfigExternalPlayerMode::Sync => ExternalPlayerLaunchMode::Sync,
        ConfigExternalPlayerMode::Spawn => ExternalPlayerLaunchMode::Spawn,
    }
}

pub(crate) fn build_media_policy(config: &Config) -> MediaPolicy {
    MediaPolicy {
        player: config.media.external_player.clone(),
        mode: map_player_mode(config.media.external_player_mode),
        player_args: config.media.external_player_args.clone(),
        allowed_players: config.media.allowed_players.clone(),
        images_enabled: matches!(config.media.image_mode, ConfigImageMode::Auto),
    }
}

pub(crate) fn build_dynamic_policy(config: &Config) -> DynamicPolicy {
    DynamicPolicy {
        mode: match config.dynamic.mode {
            ConfigDynamicMode::Off => DynamicMode::Off,
            ConfigDynamicMode::Manual => DynamicMode::Manual,
            ConfigDynamicMode::Auto => DynamicMode::Auto,
        },
        executable: config.dynamic.executable.clone(),
        wait_ms: config.dynamic.wait_ms,
        timeout_ms: config.dynamic.timeout_ms,
        // An external browser can load private subresources after a public navigation.
        // Disable the bridge when the strict public-network policy is requested.
        security_compatible: config.security.allow_private_ip,
    }
}

pub(crate) fn build_network_policy(
    config: &Config,
    allowed_schemes: Vec<String>,
) -> Result<NetworkPolicy> {
    Ok(NetworkPolicy {
        access: NetworkAccessPolicy::new(allowed_schemes, config.security.allow_private_ip)?,
        max_body_bytes: config.network.max_body_bytes,
        max_image_bytes: config.network.max_image_bytes,
        sanitize_control_chars: config.security.sanitize_control_chars,
    })
}
