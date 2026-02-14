mod browser;
mod policy;
mod startup;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};

use glance_config::{Config, JsDefault};
use glance_js::JsPolicy;

use crate::browser::Browser;
use crate::policy::{
    build_media_policy, build_network_policy, normalize_scheme_list, validate_url_for_network,
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

    #[arg(long, value_enum)]
    js: Option<JsModeArg>,

    #[arg(long = "dump-dom", value_name = "URL")]
    dump_dom: Option<String>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ValueEnum)]
enum JsModeArg {
    Off,
    Ask,
    On,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;

    let js_default = resolve_js_default(cli.js, config.startup.default_js);
    let js_policy = JsPolicy::new(
        matches!(js_default, JsDefault::On),
        config.js.allow_domains.clone(),
    );

    let (runtime, net) = build_runtime_and_net(&config)?;

    if let Some(dump_url) = cli.dump_dom {
        return run_dump_dom(
            &runtime,
            &net,
            dump_url,
            config.network.max_body_bytes,
            config.security.sanitize_control_chars,
        );
    }

    let allowed_schemes = normalize_scheme_list(&config.network.allow_schemes);
    if allowed_schemes.is_empty() {
        bail!("network.allow_schemes cannot be empty");
    }

    let media_policy = build_media_policy(&config);
    let network_policy = build_network_policy(&config, allowed_schemes);

    let initial_target = resolve_initial_target(cli.url, &config.startup.home)?;
    if let InitialTarget::Url(ref url) = initial_target {
        validate_url_for_network(
            url,
            &network_policy.allowed_schemes,
            network_policy.allow_private_ip,
        )?;
    }

    let mut browser = Browser::new(
        runtime,
        net,
        js_policy,
        config.ui.icon_mode,
        media_policy,
        network_policy,
    );

    let first = build_initial_page(&mut browser, initial_target, cli.private)?;
    glance_ui::run(first, |cmd| Ok(browser.handle_command(cmd)))
}

fn resolve_js_default(arg: Option<JsModeArg>, config_default: JsDefault) -> JsDefault {
    match arg {
        Some(JsModeArg::Off) => JsDefault::Off,
        Some(JsModeArg::Ask) => JsDefault::Ask,
        Some(JsModeArg::On) => JsDefault::On,
        None => config_default,
    }
}
