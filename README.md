# Glance

Glance is a keyboard-first, text-oriented TUI web browser for reading documentation,
articles, search results, and modern web pages without leaving the terminal. It renders the
server response immediately, then can upgrade sparse client-rendered shells with an isolated
headless Chromium DOM snapshot. Complex sign-in, OAuth, MFA, and passkey flows use a visible,
isolated Chromium companion whose authenticated DOM remains connected to the TUI.

Glance does not implement an incomplete JavaScript engine of its own. Dynamic mode delegates
page execution to an installed Chromium-family browser and clearly marks the result as
`DYNAMIC`; normal server-rendered pages stay in the faster `STATIC` mode.

## Highlights

- A responsive browser shell: focused reader view on narrow terminals, with a Page Dock for
  page context and detected images/media when more width is available.
- Low-noise browser chrome, origin/security cues, reading progress, a compact flat command bar, a
  visible scrollbar, floating activity/completion cards, and an on-demand command palette.
- Responsive navigation: requests and image downloads run without blocking the TUI.
- Automatic, non-blocking dynamic-DOM upgrades for likely JavaScript-only app shells, with a
  manual fallback and visible render-mode status.
- Automatic sign-in-link discovery with a prominent account action in both the wide Page Dock
  and compact footer, plus a non-blocking secure Chromium session for passkeys, QR sign-in,
  OAuth, CAPTCHA, and MFA.
- Continuous authenticated feeds: reaching the local reading edge advances the live Chromium
  viewport, waits asynchronously for lazy content, and appends only new readable sections and
  resources even when the site virtualizes older DOM nodes.
- Label-allowlisted `Load more`/`Show more` controls from a live page appear at the end of the
  reader and can be activated directly. This is an accidental-action guard, not a trust boundary.
- Address-or-search omnibox with a configurable search provider.
- Back/forward history with per-page scroll restoration.
- Keyboard and mouse activation for links, images, media, and simple GET forms.
- Hover target previews, release-to-activate clicks, drag scrolling, a draggable scrollbar,
  accelerated wheel scrolling, and right-click URL copying through OSC 52.
- In-page search with match highlighting and next/previous navigation.
- Correct bounded decoding for UTF-8 and legacy charsets such as EUC-KR.
- Session cookies by default, optional persistent cookies, and a real private mode.
- Redirect, response-size, scheme, and private-address policies applied to pages and images.
- Kitty/Ghostty and iTerm2 inline image support with a text fallback. Wide layouts keep the
  selected thumbnail or profile image visible in the Page Dock without forcing a modal viewer.
- A keyboard-and-mouse media picker for native audio/video, media links, HLS/DASH streams,
  and common embedded providers, with a split detail view on wide terminals and shell-free launch.
- External players start on a background worker in both `spawn` and `sync` modes. An animated
  opening card appears immediately and repeated launch requests are coalesced during startup.

## Build and run

Requirements:

- A current stable Rust toolchain
- A UTF-8 terminal
- Optional: `mpv` (or another configured executable) for media links
- Optional: Google Chrome, Chromium, or Microsoft Edge for dynamic DOM rendering

```bash
cargo build --release -p glance-app
./target/release/glance
./target/release/glance https://example.com
./target/release/glance "rust terminal browser"
```

Install under Cargo's normal binary directory:

```bash
cargo install --path crates/glance-app --locked
```

## Controls

| Key | Action |
| --- | --- |
| `o`, `Ctrl-L` | Edit the current address; plain words become a web search |
| `:` | Enter a command or address in an empty omnibox |
| `/` | Find text in the current page |
| `n`, `N` | Next/previous find match; without a find, `n` pages down |
| `j`/`k`, arrows | Move the content cursor |
| `Space`, `PageDown`/`PageUp` | Move by one page; extending a connected live feed near its edge |
| `Enter` | Open the link, image, media item, form, or safe continuation control at the cursor |
| `b` / `f` | Back / forward |
| `r` | Reload |
| `d` | Re-render the current page through headless Chromium |
| `i` | Toggle inline-image mode |
| `m` | Open the page media picker |
| `a` | Continue at the detected sign-in entry in the secure Chromium companion |
| `e` | Open the current page in the GUI browser |
| `?`, `:help` | Open the command palette |
| `y` | Copy the URL at the content cursor |
| `g` / `G` | Top / bottom |
| `Tab` | Switch address/content focus |
| `Esc` | Cancel input, close the image viewer, or clear find highlights |
| `q`, `Ctrl-C` | Quit |

Mouse controls:

- Hover an actionable item to preview its type, label, and destination.
- Left-click activates on button release, so dragging does not accidentally open a link.
- Drag in the content pane to move the cursor; drag the right scrollbar for fast navigation.
- Wheel scrolls three lines, Shift-wheel eight lines, and Ctrl-wheel one page. Near the end of an
  authenticated live page it also requests the next browser-backed feed segment.
- Right-click an actionable item to copy its URL using OSC 52; middle-click opens links in the
  GUI browser and launches media directly.
- Click the flat `●`, `◇`, or `▶` control to request a dynamic DOM, toggle images, or open the
  media picker. The `⚡` address-bar control requests a dynamic DOM; `↗` opens the page in the
  GUI browser. A detected `♙ Sign in` control opens the exact sign-in URL in an isolated Chromium
  companion; pressing it again only focuses the existing sign-in window and does not restart the
  authentication observer. Completion must remain stable across two observations, and input while
  returning to the page cannot open a second sign-in flow.
- In a wide terminal, click an image in the Page Dock to select it and keep an inline preview;
  middle-click opens the full image viewer. Middle-click media to launch it immediately, or
  right-click an item to copy its URL.

Inside the media picker, use `j`/`k`, arrows, wheel, Home/End, or the clickable controls.
Enter, Space, middle-click, or `Play` launches the selected item; `y` or right-click copies
its URL. The image viewer supports left/right or wheel navigation, clickable controls, and URL
copying as well.

The input editor supports left/right, Home/End, Delete, Backspace, `Ctrl-A`,
`Ctrl-E`, `Ctrl-U`, `Ctrl-W`, Unicode text, and bracketed paste.

Commands include `back`, `forward`, `reload`, `dynamic`/`dyn`, `login`/`auth`, `more`/`feed`,
`browser`/`gui`, `img`, `help`, `link N`, `image N`, and `media N`.

## Configuration

Default path on macOS and Linux: `~/.config/glance/config.toml`.
Every section and field is optional; omitted values use safe defaults.

```toml
[startup]
home = ""

[search]
template = "https://html.duckduckgo.com/html/?q={query}"

[network]
user_agent = "glance/0.2.0"
timeout_ms = 15000
max_redirects = 10
max_body_bytes = 4194304
max_image_bytes = 8388608
allow_schemes = ["http", "https", "about"]

[cookies]
mode = "session" # session | persistent
# path = "/custom/path/cookies.json"

[media]
image_mode = "auto"       # auto | text
external_player = "mpv"
external_player_mode = "spawn" # spawn | sync
# Arguments are passed directly, never through a shell. If {url} is omitted, it is appended.
external_player_args = ["--force-window=yes", "{url}"]
allowed_players = ["mpv"]

[dynamic]
mode = "auto" # auto | manual | off
# executable = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
wait_ms = 10000
timeout_ms = 30000

[security]
sanitize_control_chars = true
allow_private_ip = true # set false when browsing untrusted links

[ui]
icon_mode = "auto" # auto | ascii
```

Persistent HTTP cookies are stored in the platform-local data directory unless `path` is set.
The same `persistent` mode retains the isolated Chromium companion profile under Glance's local
data directory; `session` uses a randomly named temporary profile. `--private` always uses a fresh
in-memory cookie jar and temporary Chromium profile, both removed on exit.

Dynamic rendering uses a desktop-sized, fresh temporary browser profile for each page and removes
it after the DOM snapshot. The longer default settle budget is intentional: sites such as
Hackers' Pub do not expose their account navigation during the first hydration pass. The resulting
links, media, and readable controls become native Glance items.

For sign-in, Glance opens the detected account URL in a dedicated, visible Chromium profile. The
browser and operating system own the password manager, OAuth, passkey/QR, CAPTCHA, and MFA UI;
Glance never asks for or extracts those credentials. After the site returns to the original origin,
the same browser session supplies hydrated DOM snapshots for subsequent TUI navigation, so cookies
never need to be copied into the HTTP client.

On a connected session page, scrolling, Space, PageDown, End, or `G` near the TUI reading edge
advances the real Chromium viewport. Glance displays an animated `LIVE FEED` state while it waits,
then merges new text, links, images, forms, and media without discarding content removed by a
virtualized site DOM. Requests are coalesced, and accumulated content has bounded item counts to
avoid unbounded terminal-session memory growth. `:more` explicitly requests the same operation.
When a site instead requires an explicit read-only continuation action, matching controls such as
`Load more posts` appear after the readable content and can be activated with Enter or a click.
Floating cards distinguish DOM/feed work in progress from successful or failed completion.

The companion keeps Chromium's sandbox enabled, respects TLS errors, denies downloads, binds its
DevTools endpoint to `127.0.0.1` on a random port, validates that endpoint before use, and never
logs it. Profile directories are restricted to owner access, reject symlinks, and are exclusively
locked. See [docs/SECURITY.md](docs/SECURITY.md) for the threat boundary. Set `mode = "manual"` to
use only the `d` command, or `mode = "off"` to keep a strictly static reader. Dynamic and companion
modes are disabled when `security.allow_private_ip = false` because Chromium subresource requests
cannot currently be enforced by Glance's public-network-only policy.

Both external-player modes keep the TUI responsive. `spawn` reports once the player process has
started; `sync` keeps the activity state until the player exits, without blocking navigation or
other browser input.

## CLI

```text
glance [ADDRESS OR SEARCH]
  --config <path>
  --private
  --dump-dom <url>
```

`--dump-dom` is useful for scripts and parser debugging. Network responses are always
bounded by the configured byte limits, and terminal control characters are sanitized by
default.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
```

See [docs/ROADMAP.md](docs/ROADMAP.md) for the product direction, known limits, and
prioritized next work.

The current live-site and sign-in support matrix is tracked in
[docs/SITE_COMPATIBILITY.md](docs/SITE_COMPATIBILITY.md).
