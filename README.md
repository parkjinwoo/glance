# glance

A concise TUI web browser focused on practical keyboard-first browsing.

## Prerequisites

- Rust toolchain (`cargo`, `rustc`)
- Terminal with mouse and UTF-8 support
- Optional media player for external playback (default: `mpv`)

## Build

```bash
cargo build -p glance-app
```

Release build:

```bash
cargo build --release -p glance-app
```

## Run

Run with default config startup target:

```bash
cargo run -p glance-app
```

Run with a URL:

```bash
cargo run -p glance-app -- https://example.com
```

## Install Binary

Binary name is `glance`.

Install with Cargo to a custom root:

```bash
cargo install --path crates/glance-app --root "$HOME/.local"
```

Then ensure your `PATH` includes the root bin directory:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Or copy a release binary manually:

```bash
cp target/release/glance "$HOME/.local/bin/glance"
chmod +x "$HOME/.local/bin/glance"
```

Recommended locations are under your home directory (for example `~/.local/bin` or `~/bin`), but this is not required.

## Config

Default config path:

- macOS/Linux: `~/.config/glance/config.toml`

Create config directory and file:

```bash
mkdir -p "$HOME/.config/glance"
cat > "$HOME/.config/glance/config.toml" <<'TOML'
[startup]
home = ""                  # blank page by default (about:blank is also supported)
default_js = "on"          # off | ask | on

[network]
user_agent = "glance/0.1"
timeout_ms = 15000
max_redirects = 10
max_body_bytes = 4194304   # 4 MiB
max_image_bytes = 8388608  # 8 MiB
allow_schemes = ["http", "https", "about"]

[cookies]
mode = "session"           # session | persistent

[js]
allow_domains = ["example.com"]

[media]
image_mode = "auto"        # auto | text
external_player = "mpv"
external_player_mode = "sync" # sync | spawn
allowed_players = []          # empty means allow any executable name/path

[security]
sanitize_control_chars = true
allow_private_ip = true

[ui]
icon_mode = "auto"         # auto | ascii
TOML
```

Use a custom config path:

```bash
glance --config /path/to/config.toml
```

## CLI

```text
glance [URL]
  --config <path>
  --private
  --js off|ask|on
  --dump-dom <url>
```

Notes:
- If `[startup].home` is empty (`""`) or `about:blank`, Glance starts on a blank page.
- Inputs like `https:example.com` are normalized to `https://example.com`.
- Network responses are bounded by `max_body_bytes` and `max_image_bytes`.
- `sanitize_control_chars = true` strips terminal control characters from rendered page text.
- `allow_schemes` controls which URL schemes can be loaded.

## Safe Defaults

- Keep `allow_schemes = ["http", "https", "about"]`.
- Keep `sanitize_control_chars = true`.
- Use `external_player_mode = "spawn"` if you do not want media playback to block the TUI.
- Use `allowed_players = ["mpv"]` (or your preferred single player) to lock media execution.
