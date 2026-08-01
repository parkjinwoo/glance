# Live-site compatibility audit

Checked on 2026-08-01 against the public signed-out pages. These sites change independently of
Glance, so this is a smoke-test record rather than a permanent guarantee.

| Site | Reading and navigation | Search | Sign-in | Media |
| --- | --- | --- | --- | --- |
| Hackers' Pub | Automatic dynamic upgrade exposes the hydrated timeline and navigation | Link navigation | `/sign` opens in the secure companion; native WebAuthn supports platform passkeys and phone/tablet QR, then the authenticated DOM returns to the TUI | Images are extracted; linked playable media uses the media picker |
| Naver | Automatic dynamic upgrade is requested for the partial `EAGER-DATA` shell | The hydrated search control is readable; the omnibox remains a reliable fallback | `nid.naver.com` is detected and opened in the secure companion | Ordinary Naver links are no longer mislabeled as media |
| Google | The server page is usable without Chromium | The native GET search form is placed before page content | The Google Accounts URL is detected and opened in the secure companion | Images remain selectable; no false media entries |
| YouTube | The serialized app shell is automatically upgraded to a hydrated DOM | Hydrated search UI is readable; the omnibox can navigate directly | Google Accounts sign-in is detected and opened in the secure companion | Only playable `/watch`, `/shorts`, `/live`, embed, or short URLs become stream items; home/about/legal links do not |

## Authentication boundary

Glance never asks for a password, OAuth secret, passkey, CAPTCHA, or MFA code. Selecting an account
action opens the exact detected sign-in URL in an isolated, visible Chromium companion, where the
browser and operating system own the security UI. On success, TUI navigation continues through that
same browser context; cookies and WebAuthn responses are not exported to Glance's HTTP client.
