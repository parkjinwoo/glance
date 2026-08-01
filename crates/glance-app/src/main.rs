mod browser;
mod companion;
mod dynamic;
mod external;
mod policy;
mod startup;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;

use glance_config::{Config, CookieMode, default_browser_profile_path};

use crate::browser::{Browser, BrowserOptions};
use crate::companion::CompanionProfile;
use crate::policy::{
    build_dynamic_policy, build_media_policy, build_network_policy, normalize_scheme_list,
    validate_url_for_network,
};
use crate::startup::{
    InitialTarget, build_initial_page, build_runtime_and_net, resolve_initial_target, run_dump_dom,
};

#[derive(Debug, Parser)]
#[command(name = "glance", version, about = "A concise TUI web browser")]
struct Cli {
    #[arg(value_name = "URL")]
    url: Option<String>,

    #[arg(long)]
    config: Option<PathBuf>,

    #[arg(long)]
    private: bool,

    #[arg(long = "dump-dom", value_name = "URL")]
    dump_dom: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;

    let allowed_schemes = normalize_scheme_list(&config.network.allow_schemes);
    if allowed_schemes.is_empty() {
        bail!("network.allow_schemes cannot be empty");
    }

    let media_policy = build_media_policy(&config);
    let dynamic_policy = build_dynamic_policy(&config);
    let network_policy = build_network_policy(&config, allowed_schemes)?;
    let (runtime, net) = build_runtime_and_net(&config, cli.private)?;

    if let Some(dump_url) = cli.dump_dom {
        let result = run_dump_dom(
            &runtime,
            &net,
            dump_url,
            config.network.max_body_bytes,
            config.security.sanitize_control_chars,
            &network_policy.access,
        );
        let save_result = net.save_cookies();
        result?;
        return save_result;
    }

    let initial_target =
        resolve_initial_target(cli.url, &config.startup.home, &config.search.template)?;
    if let InitialTarget::Url(ref url) = initial_target {
        validate_url_for_network(url, &network_policy.access)?;
    }

    let mut browser = Browser::new(
        runtime,
        net,
        BrowserOptions {
            icon_mode: config.ui.icon_mode,
            media: media_policy,
            dynamic: dynamic_policy,
            network: network_policy,
            search_template: config.search.template.clone(),
            private: cli.private,
            companion_profile: if cli.private || matches!(config.cookies.mode, CookieMode::Session)
            {
                CompanionProfile::Ephemeral
            } else {
                CompanionProfile::Persistent(default_browser_profile_path())
            },
        },
    );

    let first = build_initial_page(&mut browser, initial_target)?;
    let ui_result = glance_ui::run(first, |cmd| Ok(browser.handle_command(cmd)));
    let shutdown_result = browser.shutdown();
    let save_result = browser.save_session();
    ui_result?;
    shutdown_result?;
    save_result
}
