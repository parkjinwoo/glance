# Live-site compatibility audit

Checked on 2026-08-02 against public pages, plus a non-mutating authenticated Hackers' Pub DOM
inspection. These sites change independently of Glance, so this is a smoke-test record rather than
a permanent guarantee.

| Site | Reading and navigation | Live interaction | Sign-in | Media |
| --- | --- | --- | --- | --- |
| Hackers' Pub | Automatic dynamic upgrade exposes the hydrated timeline and navigation | Connected sessions expose the real reply textarea and submit control, short-post composer, language suggestions, visibility/image/poll controls, share/quote menus, reaction pickers, bookmark/options controls, and portal dialogs; mutating actions require confirmation | `/sign` opens in the secure companion; native WebAuthn supports platform passkeys and phone/tablet QR, then the authenticated DOM returns to the TUI | Images are extracted; linked playable media uses the media picker |
| Naver | Automatic dynamic upgrade is requested for the partial `EAGER-DATA` shell; the hydrated search control is readable and the omnibox remains a reliable fallback | Visible supported controls are projected after a connected sign-in; site-specific transaction flows remain browser-owned | `nid.naver.com` is detected and opened in the secure companion | Ordinary Naver links are no longer mislabeled as media |
| Google | The server page and native GET search form are usable without Chromium | Connected-session controls use the generic live-control bridge; account/security prompts remain in Chromium | The Google Accounts URL is detected and opened in the secure companion | Images remain selectable; no false media entries |
| YouTube | The serialized app shell is automatically upgraded to a hydrated DOM; the omnibox can navigate directly | Connected-session visible menus and fields use the generic bridge; playback remains delegated to the media path | Google Accounts sign-in is detected and opened in the secure companion | Only playable `/watch`, `/shorts`, `/live`, embed, or short URLs become stream items; home/about/legal links do not |

## Authentication boundary

Glance never asks for a password, OAuth secret, passkey, CAPTCHA, or MFA code. Selecting an account
action opens the exact detected sign-in URL in an isolated, visible Chromium companion, where the
browser and operating system own the security UI. On success, TUI navigation continues through that
same browser context; cookies and WebAuthn responses are not exported to Glance's HTTP client.
