# Glance product direction

## Positioning

Glance should become an excellent **reader-first hybrid terminal browser**. Its primary jobs are
searching the web, reading static or hydrated content, following links, inspecting media, and
performing small safe interactions without disrupting a terminal workflow. Complex identity and
application flows use an explicit, isolated Chromium companion instead of a misleading partial
clone.

That boundary is deliberate. A general JavaScript runtime without a complete DOM, CSS,
layout, accessibility, storage, and security model produces misleading results. Dynamic-web
support therefore uses a mature, isolated Chromium process and exposes the resulting DOM
through the existing TUI contracts.

## Current 0.2 baseline

- Responsive reader layout with a wide-screen media dock, persistent selected-image preview,
  reading progress, and low-noise icon-led browser chrome that degrades cleanly on narrow terminals
- Command palette and contextual command bar instead of relying on memorized shortcuts alone
- Non-blocking page and image requests
- Static HTML extraction and terminal-safe rendering
- Address/search omnibox and configurable search template
- Link, image, media, and single-text-field GET-form interactions
- Hover previews, safe release-to-activate clicks, drag/wheel scrolling, and URL copying
- Responsive media library for native audio/video sources, common files, HLS/DASH, and known
  embeds, plus Page Dock shortcuts for detected assets
- Non-blocking external-player startup with visible activity state and duplicate-launch suppression
- Shell-free configurable external-player arguments and image/media carousel controls
- In-page search
- Session, persistent, and private cookie modes
- Redirect and response-size policy enforcement
- HTTP charset decoding and terminal control-character sanitization
- Inline image support with text fallback
- Static-first automatic Chromium DOM snapshots for sparse client-rendered shells
- Manual dynamic re-rendering and visible static/upgrading/dynamic status
- Accessible names for icon-only links
- Sign-in discovery and responsive account actions, backed by a persistent-in-process Chromium
  session with native passkey, QR, OAuth, CAPTCHA, and MFA UI
- Owner-only browser profiles, exclusive profile locking, loopback-only random DevTools endpoints,
  TLS validation, sandbox preservation, download denial, and private-profile cleanup
- Authenticated TUI navigation using the same browser context without exporting raw cookies
- Browser-backed continuous feeds with edge-triggered prefetch, virtual-DOM snapshot accumulation,
  duplicate-request suppression, bounded memory, and visible non-blocking progress
- Allowlisted live-page continuation buttons, with label revalidation immediately before activation
- Floating in-progress/completion feedback for dynamic DOM, feed, and media work
- Repeated sign-in activation focuses the existing window without resetting authentication polling;
  stable completion confirmation and return-navigation coalescing prevent duplicate flows
- GUI-browser continuation from the address bar, footer, mouse middle-click, key, and command
- Script/style payload suppression in fallback text and playable-URL-aware media classification
- Live compatibility checks for Hackers' Pub, Naver, Google, and YouTube

## Priorities

### P0 — dependable daily reading

- Replace selector-based block collection with a single DOM-tree traversal so nested content
  is ordered once and inline links remain interactive inside paragraphs.
- Add integration tests with a local HTTP fixture server for redirects, cookies, charset
  decoding, oversized bodies, malformed HTML, and cancellation races.
- Add download support with explicit destination confirmation and MIME-aware filenames.
- Add bookmarks and durable browsing history in a small versioned local database.
- Maintain a small live-site compatibility audit alongside deterministic local fixtures.

### P1 — better interaction and accessibility

- Support multi-field GET forms, checkboxes, radio groups, and selects through a form overlay.
- Add opt-in POST forms with password masking, origin warnings, and redirect-correct method
  handling.
- Extract landmarks, language, headings, and accessible names; add heading/link navigation.
- Add reader-mode heuristics that suppress navigation chrome and duplicated boilerplate.
- Add configurable keymaps, colors, and CJK width behavior.

### P2 — deeper dynamic web bridge

- Extend the authenticated DevTools session from read-only continuation controls to narrowly
  scoped, origin-checked form actions; keep security prompts and credential entry inside Chromium.
- Add an explicit session-management screen for profile retention, revocation, and site sign-out
  without exporting raw cookie material.
- Add stable item-identity adapters for highly virtualized feeds where text-level generic
  deduplication cannot distinguish two intentionally identical posts.
- Reuse the same TUI history, search, security, and rendering contracts where possible.

## Quality gates

Every release should pass formatting, warnings-as-errors Clippy, all workspace tests, and
network fixture integration tests on macOS, Linux, and Windows. Dependency updates should be
automated, reviewed for breaking changes, and accompanied by `cargo audit` or an equivalent
RustSec check in CI.
