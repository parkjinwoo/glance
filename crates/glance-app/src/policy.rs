use anyhow::{Result, bail};
use url::{Host, Url};

use glance_config::{Config, ExternalPlayerMode as ConfigExternalPlayerMode};
use glance_media::ExternalPlayerLaunchMode;

#[derive(Debug, Clone)]
pub(crate) struct NetworkPolicy {
    pub(crate) allowed_schemes: Vec<String>,
    pub(crate) allow_private_ip: bool,
    pub(crate) max_body_bytes: usize,
    pub(crate) max_image_bytes: usize,
    pub(crate) sanitize_control_chars: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct MediaPolicy {
    pub(crate) player: String,
    pub(crate) mode: ExternalPlayerLaunchMode,
    pub(crate) allowed_players: Vec<String>,
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

pub(crate) fn validate_url_for_network(
    url: &Url,
    allowed_schemes: &[String],
    allow_private_ip: bool,
) -> Result<()> {
    let scheme = url.scheme().to_ascii_lowercase();
    if !allowed_schemes.iter().any(|s| s == &scheme) {
        bail!("scheme '{}' is not allowed by config", url.scheme());
    }
    if !allow_private_ip && is_private_host(url) {
        bail!("private or loopback target is blocked by config");
    }
    Ok(())
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
        allowed_players: config.media.allowed_players.clone(),
    }
}

pub(crate) fn build_network_policy(config: &Config, allowed_schemes: Vec<String>) -> NetworkPolicy {
    NetworkPolicy {
        allowed_schemes,
        allow_private_ip: config.security.allow_private_ip,
        max_body_bytes: config.network.max_body_bytes,
        max_image_bytes: config.network.max_image_bytes,
        sanitize_control_chars: config.security.sanitize_control_chars,
    }
}

fn is_private_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Some(Host::Ipv6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
        Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}
