use std::io::{Write, stdout};
use std::process::Command;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crossterm::cursor::MoveTo;
use crossterm::execute;
use image::ImageFormat;
use url::Url;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ExternalPlayerLaunchMode {
    Sync,
    Spawn,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ImageBackend {
    Kitty,
    ITerm2,
    Sixel,
    TextFallback,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct InlineImagePlacement {
    pub backend: ImageBackend,
    pub image_id: u32,
    pub col: u16,
    pub row: u16,
    pub width_cells: u16,
    pub height_cells: u16,
}

pub fn detect_image_backend() -> ImageBackend {
    let term = std::env::var("TERM")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let term_program = std::env::var("TERM_PROGRAM")
        .unwrap_or_default()
        .to_ascii_lowercase();
    detect_image_backend_from_env(&term, &term_program)
}

fn detect_image_backend_from_env(term: &str, term_program: &str) -> ImageBackend {
    let term = term.to_ascii_lowercase();
    let term_program = term_program.to_ascii_lowercase();

    if term.contains("xterm-kitty") {
        return ImageBackend::Kitty;
    }
    // Ghostty supports kitty graphics protocol; treat it as kitty-compatible.
    if term.contains("ghostty") || term_program.contains("ghostty") {
        return ImageBackend::Kitty;
    }
    if term_program.contains("iterm") {
        return ImageBackend::ITerm2;
    }
    if term.contains("sixel") {
        return ImageBackend::Sixel;
    }

    ImageBackend::TextFallback
}

pub fn image_backend_label(backend: ImageBackend) -> &'static str {
    match backend {
        ImageBackend::Kitty => "kitty",
        ImageBackend::ITerm2 => "iterm2",
        ImageBackend::Sixel => "sixel",
        ImageBackend::TextFallback => "text",
    }
}

pub fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let size = imagesize::blob_size(bytes).ok()?;
    Some((size.width as u32, size.height as u32))
}

pub fn compute_aspect_fit(
    orig_w: u32,
    orig_h: u32,
    max_w_cells: u16,
    max_h_cells: u16,
    cell_px_w: u16,
    cell_px_h: u16,
) -> (u16, u16) {
    if orig_w == 0 || orig_h == 0 || max_w_cells == 0 || max_h_cells == 0 {
        return (1, 1);
    }

    let max_w_px = (max_w_cells as f64) * (cell_px_w as f64);
    let max_h_px = (max_h_cells as f64) * (cell_px_h as f64);
    let scale = (max_w_px / (orig_w as f64)).min(max_h_px / (orig_h as f64));

    let fit_w_px = ((orig_w as f64) * scale).max(cell_px_w as f64);
    let fit_h_px = ((orig_h as f64) * scale).max(cell_px_h as f64);

    let fit_w_cells = (fit_w_px / (cell_px_w as f64)).floor() as u16;
    let fit_h_cells = (fit_h_px / (cell_px_h as f64)).floor() as u16;

    (
        fit_w_cells.max(1).min(max_w_cells),
        fit_h_cells.max(1).min(max_h_cells),
    )
}

pub fn render_inline_image_at(
    image_bytes: &[u8],
    placement: InlineImagePlacement,
) -> Result<(), String> {
    if matches!(
        placement.backend,
        ImageBackend::TextFallback | ImageBackend::Sixel
    ) {
        return Ok(());
    }

    execute!(stdout(), MoveTo(placement.col, placement.row))
        .map_err(|e| format!("cursor move failed: {e}"))?;

    match placement.backend {
        ImageBackend::Kitty => render_kitty(
            &ensure_kitty_png_payload(image_bytes)?,
            placement.image_id,
            placement.width_cells,
            placement.height_cells,
        ),
        ImageBackend::ITerm2 => {
            render_iterm2(image_bytes, placement.width_cells, placement.height_cells)
        }
        ImageBackend::Sixel | ImageBackend::TextFallback => Ok(()),
    }
}

fn ensure_kitty_png_payload(image_bytes: &[u8]) -> Result<Vec<u8>, String> {
    const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if image_bytes.starts_with(&PNG_SIG) {
        return Ok(image_bytes.to_vec());
    }

    let img = image::load_from_memory(image_bytes)
        .map_err(|e| format!("unsupported image format for kitty backend: {e}"))?;
    let mut png = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut png);
    img.write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| format!("failed to transcode image for kitty backend: {e}"))?;
    Ok(png)
}

pub fn clear_inline_image(placement: InlineImagePlacement) -> Result<(), String> {
    match placement.backend {
        ImageBackend::Kitty => clear_kitty(placement.image_id),
        ImageBackend::ITerm2 => clear_iterm2(placement),
        ImageBackend::Sixel | ImageBackend::TextFallback => Ok(()),
    }
}

fn render_kitty(
    image_bytes: &[u8],
    image_id: u32,
    width_cells: u16,
    height_cells: u16,
) -> Result<(), String> {
    let payload = STANDARD.encode(image_bytes);
    let mut out = stdout();

    let chunk_size = 4096;
    let mut first = true;

    let chunks: Vec<&[u8]> = payload.as_bytes().chunks(chunk_size).collect();
    for (idx, chunk) in chunks.iter().enumerate() {
        let m = if idx + 1 == chunks.len() { 0 } else { 1 };

        if first {
            write!(
                out,
                "\x1b_Ga=T,f=100,t=d,q=2,C=1,i={},c={},r={},m={};{}\x1b\\",
                image_id,
                width_cells,
                height_cells,
                m,
                String::from_utf8_lossy(chunk)
            )
            .map_err(|e| format!("kitty write failed: {e}"))?;
            first = false;
        } else {
            write!(
                out,
                "\x1b_Gi={},m={};{}\x1b\\",
                image_id,
                m,
                String::from_utf8_lossy(chunk)
            )
            .map_err(|e| format!("kitty write failed: {e}"))?;
        }
    }

    out.flush().map_err(|e| format!("kitty flush failed: {e}"))
}

fn clear_kitty(image_id: u32) -> Result<(), String> {
    let mut out = stdout();
    write!(out, "\x1b_Ga=d,d=I,q=2,i={}\x1b\\", image_id)
        .map_err(|e| format!("kitty clear failed: {e}"))?;
    out.flush()
        .map_err(|e| format!("kitty clear flush failed: {e}"))
}

fn render_iterm2(image_bytes: &[u8], width_cells: u16, height_cells: u16) -> Result<(), String> {
    let payload = STANDARD.encode(image_bytes);
    let mut out = stdout();

    write!(
        out,
        "\x1b]1337;File=inline=1;width={};height={};preserveAspectRatio=1:{}\x07",
        width_cells, height_cells, payload
    )
    .map_err(|e| format!("iterm2 write failed: {e}"))?;

    out.flush().map_err(|e| format!("iterm2 flush failed: {e}"))
}

fn clear_iterm2(placement: InlineImagePlacement) -> Result<(), String> {
    let mut out = stdout();
    for y in 0..placement.height_cells {
        execute!(out, MoveTo(placement.col, placement.row.saturating_add(y)))
            .map_err(|e| format!("iterm2 clear cursor move failed: {e}"))?;
        write!(out, "{}", " ".repeat(placement.width_cells as usize))
            .map_err(|e| format!("iterm2 clear write failed: {e}"))?;
    }
    out.flush()
        .map_err(|e| format!("iterm2 clear flush failed: {e}"))
}

pub fn launch_external_player(player: &str, media_url: &Url) -> Result<(), String> {
    launch_external_player_with_options(player, media_url, ExternalPlayerLaunchMode::Sync, &[])
}

pub fn launch_external_player_with_options(
    player: &str,
    media_url: &Url,
    mode: ExternalPlayerLaunchMode,
    allowed_players: &[String],
) -> Result<(), String> {
    let player = player.trim();
    if player.is_empty() {
        return Err("external player command is empty".to_string());
    }
    if player.contains(char::is_whitespace) {
        return Err(
            "external player must be a single executable name or path (no spaces)".to_string(),
        );
    }
    if !allowed_players.is_empty() && !allowed_players.iter().any(|p| p == player) {
        return Err(format!(
            "external player '{player}' is not allowed by policy"
        ));
    }

    let mut cmd = Command::new(player);
    cmd.arg(media_url.as_str());

    match mode {
        ExternalPlayerLaunchMode::Sync => {
            let status = cmd
                .status()
                .map_err(|e| format!("failed to launch player '{player}': {e}"))?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("player exited with status: {status}"))
            }
        }
        ExternalPlayerLaunchMode::Spawn => {
            cmd.spawn()
                .map_err(|e| format!("failed to spawn player '{player}': {e}"))?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExternalPlayerLaunchMode, ImageBackend, compute_aspect_fit, detect_image_backend_from_env,
        image_dimensions, launch_external_player_with_options,
    };
    use url::Url;

    #[test]
    fn recognizes_ghostty_as_kitty_compatible() {
        let backend = detect_image_backend_from_env("xterm-ghostty", "ghostty");
        assert_eq!(backend, ImageBackend::Kitty);
    }

    #[test]
    fn recognizes_iterm2() {
        let backend = detect_image_backend_from_env("xterm-256color", "iTerm.app");
        assert_eq!(backend, ImageBackend::ITerm2);
    }

    #[test]
    fn falls_back_to_text() {
        let backend = detect_image_backend_from_env("xterm-256color", "");
        assert_eq!(backend, ImageBackend::TextFallback);
    }

    #[test]
    fn computes_aspect_fit_preserving_ratio() {
        let (w, h) = compute_aspect_fit(1600, 900, 40, 20, 8, 16);
        assert!(w <= 40);
        assert!(h <= 20);
        assert!(w > h);
    }

    #[test]
    fn can_parse_png_dimensions() {
        // 1x1 transparent PNG
        let bytes = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        assert_eq!(image_dimensions(&bytes), Some((1, 1)));
    }

    #[test]
    fn rejects_player_with_whitespace() {
        let url = Url::parse("https://example.com/v.mp4").expect("url");
        let err = launch_external_player_with_options(
            "mpv --fs",
            &url,
            ExternalPlayerLaunchMode::Sync,
            &[],
        )
        .expect_err("must reject whitespace");
        assert!(err.contains("single executable"));
    }

    #[test]
    fn rejects_player_not_in_allowlist() {
        let url = Url::parse("https://example.com/v.mp4").expect("url");
        let err = launch_external_player_with_options(
            "vlc",
            &url,
            ExternalPlayerLaunchMode::Sync,
            &["mpv".to_string()],
        )
        .expect_err("must reject not-allowed player");
        assert!(err.contains("not allowed"));
    }
}
