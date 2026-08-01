use std::ffi::OsString;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};
use url::Url;

pub(crate) fn launch_system_browser(url: &Url) -> Result<u32> {
    ensure!(
        matches!(url.scheme(), "http" | "https"),
        "GUI browser handoff only supports HTTP(S) URLs"
    );

    let (program, args) = system_browser_command(url);
    let child = Command::new(&program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to open the system browser with {program}"))?;
    Ok(child.id())
}

fn system_browser_command(url: &Url) -> (String, Vec<OsString>) {
    #[cfg(target_os = "macos")]
    {
        return (
            "/usr/bin/open".to_string(),
            vec![OsString::from(url.as_str())],
        );
    }

    #[cfg(target_os = "windows")]
    {
        return (
            "explorer.exe".to_string(),
            vec![OsString::from(url.as_str())],
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return ("xdg-open".to_string(), vec![OsString::from(url.as_str())]);
    }

    #[allow(unreachable_code)]
    ("xdg-open".to_string(), vec![OsString::from(url.as_str())])
}

#[cfg(test)]
mod tests {
    use super::system_browser_command;
    use url::Url;

    #[test]
    fn browser_handoff_passes_the_url_as_one_argument() {
        let url = Url::parse("https://example.com/path?q=a%20b&x=$(ignored)").expect("url");
        let (_program, args) = system_browser_command(&url);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0], url.as_str());
    }
}
