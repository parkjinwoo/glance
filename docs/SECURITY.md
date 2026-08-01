# Browser session security

Glance treats the Chromium companion as a credential boundary. Passwords, passkeys, QR exchanges,
CAPTCHAs, OAuth secrets, MFA codes, cookies, and WebAuthn responses remain inside Chromium. Glance
receives only the final URL and serialized DOM needed for terminal rendering.

## Defaults

- Chromium's process sandbox remains enabled; Glance never adds `--no-sandbox`.
- Certificate errors are not ignored.
- DevTools binds to `127.0.0.1` on a browser-selected random port. Glance rejects a returned
  endpoint that is not loopback, does not contain an explicit port, or embeds credentials.
- The endpoint and its random browser token are never written to application logs or UI state.
- Automatic downloads are denied for the controlled browser context.
- Direct navigations are limited to the configured HTTP(S) network policy. Companion mode is
  unavailable under the strict public-only policy because Chromium subresource requests cannot be
  filtered with the same guarantees.
- Persistent browser profiles are directories with owner-only permissions and an owner-only
  advisory lock file. Symlink profile paths are rejected and concurrent Glance processes cannot
  share the profile.
- Session and private profiles use randomly named temporary directories. They are closed and
  removed on normal exit; Chromium is owned by the Glance process so normal shutdown does not leave
  a reusable debugging session running.
- Continuous-feed updates execute a fixed, bundled scroll/measurement script. Page-provided code is
  not interpolated into that script, overlapping DOM snapshots are deduplicated, and accumulated
  document/resource counts are bounded to prevent unbounded TUI memory growth.
- Continuation controls are tagged only by a fixed multilingual allowlist of read-more semantics.
  Before activation, the live element is checked again for visibility, enabled state, and the same
  label allowlist. Existing page-supplied tags are cleared and only supported button roles are
  accepted. This is a guard against accidental activation, not an authorization boundary: a
  malicious site can lie about a button's label, so activate controls only on sites you trust.
- General live controls exist only in an authenticated, connected companion session. Glance first
  removes every page-supplied control tag, then marks visible supported fields and button roles
  itself. Static responses and one-shot headless snapshots have these tags stripped, so server HTML
  cannot opt itself into live execution.
- Password, hidden, file, and other sensitive or unsupported inputs are not projected into the TUI.
  Ordinary text/textarea values, checked state for native checkbox/radio inputs, and native
  single-select options may be serialized into Glance's in-memory page model so they can be edited.
  Custom comboboxes are exposed only through their browser-backed disclosure/listbox controls;
  credentials and browser-owned security prompts remain inside Chromium.
- A disclosure such as a menu, dialog, quote editor, reaction picker, or cancel control can be
  opened directly. Generic actions and submit controls require a second explicit confirmation.
  Immediately before editing or clicking, Glance re-resolves the companion-assigned identifier and
  checks the expected kind and native field type, accessible label, visibility, enabled state,
  current page, and origin. Select values must match a currently enabled option. Values and labels
  are JSON-encoded before entering the fixed script rather than interpolated as executable
  JavaScript. Visually hidden checkbox/radio inputs are accepted only when their associated label
  is itself visible.
- Confirmation prevents an accidental click; it is not an authorization boundary. A malicious
  page controls its accessible labels and event handlers, and a click may have site-specific side
  effects. Use live actions only on sites you trust and inspect the shown label and context.
- Sign-in completion requires two consecutive observations on the same origin and path. Repeated
  activation focuses the existing authentication window, while input during authenticated return
  navigation is coalesced instead of starting another flow.

## Persistence

`cookies.mode = "session"` is the default and discards the companion profile at exit.
`cookies.mode = "persistent"` retains a dedicated Glance browser profile in the platform-local data
directory. It is not the user's default Chrome profile and Glance never attaches DevTools to that
default profile. `--private` always overrides persistence.

The strongest remaining local threat is another malicious process already running as the same OS
user. Filesystem permissions protect against other users, while the random loopback endpoint and
unguessable DevTools path reduce accidental exposure; they are not a sandbox against a compromised
user account. Use `--private` on shared or untrusted systems.
