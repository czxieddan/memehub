# meme-web

Embedded meme gallery and narrow public HTTP proxy. HTML, CSS, JavaScript, icons, and locale resources are compiled into the binary with `include_str!`. Building and serving the interface requires no Node packages, CDN, or external fonts.

## Configuration

```toml
[web]
listen = "127.0.0.1:7410"
core_url = "http://127.0.0.1:7413"
github_url = "http://127.0.0.1:7412"
source_url = "https://example.org/your-project"
```

Start the listener with `meme_web::run(config_path, shutdown_receiver).await`. The listener and upstream addresses are required. An omitted or empty `source_url` displays a notice on `/source`.

Only `[web]` is read. Upstream addresses accept HTTP or HTTPS and reject credentials, query parameters, and fragments. Public paths are appended to any configured path prefix. Source links accept HTTP or HTTPS and are escaped in both HTML attributes and text. A shutdown signal of `true`, or closing its sender, stops new connections and lets active requests finish.

## Public routes

| Web path | Destination |
| --- | --- |
| `GET /api/gallery` | core `/v1/gallery`; only `query`, `cursor`, and `limit` are forwarded |
| `GET /api/images/{hash}` | core `/v1/images/{hash}`; hash must be 64 lowercase hexadecimal characters |
| `GET /api/me` | core `/v1/me` |
| `GET /api/submissions` | core `/v1/submissions` |
| `POST /api/submissions` | core `/v1/submissions`; multipart body is forwarded unchanged |
| `GET /auth/github` | github `/auth/github` |
| `GET /auth/callback` | github `/auth/callback`; OAuth query parameters are preserved |
| `POST /auth/logout` | github `/auth/logout` |
| `POST /webhooks/github` | github `/webhooks/github`; raw bytes and allowed GitHub headers are preserved |

`/` serves the gallery, `/source` serves the deployment’s configured source link, and `/health/live` returns `{"status":"ok"}`. Static routes serve `/assets/app.css`, `/assets/app.js`, `/assets/i18n.js`, `/assets/icon.svg`, and `/assets/locales/{name}`. The locale route accepts only `en.json`, `zh-CN.json`, `ja.json`, `ko.json`, and `ru.json`; other names return 404. Unregistered paths never reach an upstream.

Ordinary requests forward only Cookie, Origin, X-CSRF-Token, and Content-Type. Webhooks forward only the explicit GitHub metadata allowlist and Content-Type. Client Authorization, internal credentials, hop-by-hop headers, and headers named by Connection are excluded. Upstream status, Set-Cookie, Location, Content-Type, Cache-Control, and X-Content-Type-Options are preserved.

The proxy never follows redirects. Authentication redirects may use a same-site relative path, an HTTP(S) address matching the request Host, or `https://github.com`; others return 502. Reverse proxies must preserve the public Host, and the GitHub service must use the same public address.

Requests are limited to 25 MiB, ordinary responses to 8 MiB, and image responses to 5 MiB. Bodies are counted while reading, including responses without Content-Length. Connection timeout is 3 seconds; upstream requests and incoming body reads each have a 30-second timeout.

## Interface and localization

The gallery supports name search, cursor pagination, and original-image viewing. Startup reads `/api/me` once and obtains the CSRF token from that response. Each upload sends `name`, `display_name`, `file`, and `consent=true`. The client checks PNG, JPEG, GIF, WebP, and the 5 MiB limit; core remains responsible for final validation.

My submissions displays processing status, localized failure guidance, creation time, and safe GitHub pull-request links, with manual refresh. Unknown status codes display a translated “Unknown status” label. Creation times accept Unix seconds or date strings and use the selected locale and browser time zone.

All interface copy lives in `assets/locales/`: English (`en`), Simplified Chinese (`zh-CN`), Japanese (`ja`), Korean (`ko`), and Russian (`ru`). HTML placeholders render escaped English from the same `en.json` catalog. The external i18n module receives that catalog from the asset handler, so English dynamic messages require no additional request.

The browser selects the first supported language in `navigator.languages`, with `navigator.language` as a final preference and English as fallback. Regional English, Japanese, Korean, and Russian tags use their base language. Chinese variants use the available Simplified Chinese catalog. Invalid tags are skipped. Locale changes in the browser update the page’s text, attributes, title, language, numbers, plurals, and dates without resetting an active form. A failed, malformed, or timed-out locale request leaves English available. A missing translated entry falls back to English.

Translations and interpolated values are applied with `textContent` or an explicit safe attribute allowlist. User-provided names remain user content. Known service error codes select translated messages; legacy or unknown errors use localized HTTP-status guidance rather than displaying unlocalized server text. Proxy-generated English error messages also come from `en.json`.

Controls include visible keyboard focus, arrow/Home/End tab navigation, native dialogs with focus containment, localized form errors, and reduced-motion styles. Production assets contain no demo data or external translation dependencies.
