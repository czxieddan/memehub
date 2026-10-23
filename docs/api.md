# HTTP/JSON API v1

Business crates communicate through HTTP without Cargo dependencies on each other. `meme-web` explicitly maps public routes; internal administration routes are not proxied. Every service provides `GET /health/live`.

## modular-db-api

All routes except health require `Authorization: Bearer <database principal token>`. Principals are scoped to namespaces; administrators can access all namespaces and create backups. The database exposes generic records. Records are limited to 64 KiB. Pages default to 100 records, with a maximum of 500, sorted by key; the cursor is the last returned key.

| Method and path | Contract |
| --- | --- |
| `GET /v1/namespaces/{namespace}/records?cursor=&limit=&prefix=` | `{items:[{key,version,value}],next_cursor}` |
| `GET /v1/namespaces/{namespace}/records/{key}` | `{key,version,value}`; 404 when absent |
| `PUT /v1/namespaces/{namespace}/records/{key}` | `{value,expected_version?}` |
| `DELETE /v1/namespaces/{namespace}/records/{key}?expected_version=` | Conditional deletion |
| `GET /v1/namespaces/{namespace}/indexes` | Definitions as `{items:[{name,pointer}]}` |
| `PUT /v1/namespaces/{namespace}/indexes/{name}` | `{pointer:"/field/child"}`; rebuilds the index transactionally |
| `DELETE /v1/namespaces/{namespace}/indexes/{name}` | Removes the definition and indexed values |
| `POST /v1/namespaces/{namespace}/indexes/{name}/query` | `{value:scalar,cursor?,limit?}`; returns a record page |
| `POST /v1/transactions` | `{operations:[...]}`; 1–10000 operations, all committed or all rolled back |
| `POST /v1/admin/backups` | Creates an encrypted backup; returns `{backup:"random-name.rnmdb"}` |

Transaction operations are `{op:"put",namespace,key,value,expected_version?}`, `{op:"delete",namespace,key,expected_version?}`, and `{op:"check",namespace,key,expected_version}`. Omitted versions are unconditional; 0 requires absence; any other version must match exactly. Conflicts return 409. Deletions retain internal version tombstones, so recreating a record does not reuse its previous version. Errors use `{error:text}`.

Callers declare scalar indexes with JSON Pointers. Strings, numbers, booleans, and null are indexed; missing fields, arrays, and objects are ignored. Queries match the JSON type and serialized value exactly: `1`, `1.0`, and `"1"` are distinct. Results are sorted by record key and use the same pagination rules as record lists. A namespace may declare up to 32 indexes; creating or redefining one may backfill up to 100000 records. Index names follow namespace naming rules. Record and index changes share a transaction, and namespace authorization applies to indexes.

## meme-github

`/v1/*` requires the separate GitHub service token. Browser sessions use the HttpOnly, SameSite=Lax `memehub_session` cookie; HTTPS configuration adds Secure.

An identity and its sessions share an authorization generation. Ordinary login preserves valid sessions. Revocation invalidates all sessions in that generation, and reauthorization cannot revive them. Identity and session creation use a conditional transaction; concurrent identity changes reject the login. Legacy sessions without an authorization generation require a new login.

| Method and path | Contract |
| --- | --- |
| `GET /auth/github` | Stores one-time OAuth state and a PKCE verifier, then redirects to GitHub |
| `GET /auth/callback?code=&state=` | Validates state, exchanges the code, saves a verified identity, and creates a local session |
| `POST /auth/logout` | Checks Origin and `X-CSRF-Token`, then deletes the session |
| `GET /auth/avatar` | Current session user's raster avatar; cookie authentication, no service token; exposed only as `/api/avatar` by the web host |
| `POST /webhooks/github` | Verifies `X-Hub-Signature-256` over the raw body and deduplicates by delivery ID |
| `GET /v1/session` | `{id,login,email,contribution_email,csrf_token}`; 401 for an invalid session |
| `POST /v1/changes` | Creates an App branch and PR |
| `GET /v1/pulls/{number}?repository=OWNER/REPO` | PR status, including `state`, `merged`, and `url` |
| `GET /v1/pulls?repository=OWNER/REPO&page=1` | `{items,next_page}` for reconciliation |
| `GET /v1/content?repository=&path=&reference=` | `{content_base64}`; 404 when absent |
| `GET /v1/events?cursor=` | Persisted events as `{items:[{cursor,event}],next_cursor}` |

A change request contains `operation_id` (a UUID idempotency key), `repository`, `base_branch`, `title`, `body`, `message`, optional `coauthor_user_id`, and `files:[{path,content_base64}]`. Configuration restricts repositories and path prefixes. Attribution comes from the saved authorized identity. The response is `{operation_id,repository,number,url,branch}`. Base64 is used for transport and is not stored in the database.

The avatar endpoint accepts no query parameters and rejects HEAD with 405. It reads the current identity through the normal session/revocation checks, then fetches only `https://avatars.githubusercontent.com/u/{positive numeric user id}?s=96&v=4`. The dedicated HTTPS client sends no session cookie, service token, or OAuth token, uses no proxy or redirects, and has a 3-second connect timeout. The GitHub service limits the whole operation to 8 seconds. Only a 200 response containing at most 256 KiB of PNG, JPEG, GIF, or WebP with matching MIME, signature, and raster framing is accepted; dimensions must be 1–1024 pixels per edge. Encoded, truncated, or invalid responses fail with 502; timeouts return 504 and invalid sessions return 401. GET and HEAD responses use `Cache-Control: private, no-store` and `nosniff`, including authentication and upstream errors. The browser can fall back to a local icon; the host's `img-src 'self' blob:` CSP and `/api/me` contract do not change.

## meme-core

| Method and path | Contract |
| --- | --- |
| `GET /v1/gallery?section=all&query=&cursor=&limit=` | `{items:[{manifest_hash,uploader_id,name,display_name}],next_cursor}`; sections `all`, `latest`, `popular`, `featured`; default page size 40, maximum 100 |
| `GET /v1/images/{hash}` | Published image with the correct Content-Type and nosniff |
| `GET /v1/memes/{hash}` | `{item:{manifest_hash,uploader_id,name,display_name}}`; 404 for invalid or unpublished hashes |
| `POST /v1/memes/{hash}/view` | `{recorded:boolean}`; requires the configured public Origin; 404 for invalid or unpublished hashes |
| `GET /v1/policy` | `{version:string,required_claims:[string,...]}`; defaults to `{"version":"","required_claims":[]}` |
| `GET /v1/me` | Session identity; 401 for an invalid session |
| `GET /v1/submissions` | Current user's submission statuses |
| `POST /v1/submissions` | Multipart `name`, `display_name`, `file`, `consent=true`, and configured `policy_version` / `policy_claims`; requires a valid cookie, Origin, and `X-CSRF-Token` |
| `GET /v1/admin/cache` | Policy, usage, and statistics |
| `PUT /v1/admin/cache` | Persists and immediately applies `{max_bytes,max_entries,ttl_seconds}` |
| `GET /v1/admin/gallery` | `{featured:[hash,...]}`; defaults to an empty list |
| `PUT /v1/admin/gallery` | Replaces the ordered featured list with `{featured:[hash,...]}`; at most 500 unique, valid, currently published hashes; 40 KiB request limit |
| `POST /v1/admin/sync` | Fetches and validates the repository index, then switches publication transactionally |
| `GET /v1/admin/submissions` | Submission tasks |
| `GET /v1/admin/pulls` | Reconciled web and external PRs |
| `POST /v1/admin/submissions/{id}/retry` | Retries a failed PR creation task |

All `/v1/admin/*` routes require the core administrator token. Submission states progress through `pending`, `retrying`, `pr_open`, `awaiting_publish`, and `published`, or enter `failed` or `closed`. `merged=true` alone does not indicate publication: the valid index must contain the entry.

Gallery search matches `name` and `display_name` case-insensitively, with a maximum query length of 256 UTF-8 bytes. Page sizes are clamped to 1-100. The default `all` section preserves ascending manifest-hash order and the last returned hash as its cursor. An empty cursor starts a page; malformed hashes and unknown sections return 400. The other sections return opaque URL-safe cursors authenticated with HMAC-SHA256 using the core administrator secret and bound to the section, normalized query, and current publication, catalog, and featured-configuration versions. Malformed, edited, mismatched, or stale cursors return 400; restart pagination without a cursor after publication, a counted view, a featured-list change, or administrator-secret rotation. Limit changes are allowed between pages.

`latest` sorts by the time the core first discovers a published hash, descending, then by hash ascending. Backfilled entries receive their discovery time during synchronization; the core does not infer historical publication dates. `popular` sorts by counted detail-view signals descending, then discovery time descending and hash ascending. Reading a detail or image does not count a view. The view endpoint counts at most once per hash per 30 seconds across all visitors and service restarts, saturating at 4294967295 views. Missing, multiple, or mismatched Origin headers return 400. No IP address, visitor ID, session, or user identity is stored for views. `featured` follows the administrator's exact list order and includes only currently published hashes. An empty list produces an empty section; removed hashes in the configuration are omitted.

Publication supports at most 500 entries. The four-field `corememes` schema and public item/detail objects remain unchanged. `corestate/gallery_catalog` is one bounded map of hash to `[first_seen,view_count,last_counted_view]`, using Unix seconds. Publication and catalog changes commit in the same database transaction: retained hashes preserve their metadata, removed hashes lose it, and newly discovered hashes start at zero views. Synchronizing an unchanged legacy publication backfills missing metadata. `coreconfig/gallery` stores the featured configuration. These records use the existing core namespace permissions. Ordered sections and view recording require catalog synchronization when upgrading an old installation.

Use `memehub admin gallery show` to inspect featured configuration, `memehub admin gallery featured HASH...` to replace it in argument order, and `memehub admin gallery featured` to clear it. These commands use the existing administrator token and apply immediately.

The optional `[core]` settings `upload_policy_version = ""` and `upload_policy_claims = []` preserve uploads using only the existing `consent=true`. A configured version requires 1-8 unique claims. Versions and claim IDs contain 1-64 ASCII letters, digits, underscores, or hyphens. Empty versions cannot have claims. Configure them with `memehub admin config set core.upload_policy_version VERSION` and `memehub admin config set core.upload_policy_claims '["claim_a","claim_b"]'`, then restart the core. The backend serves IDs only; the frontend supplies policy text.

When a policy is configured, uploads must include its exact `policy_version` and `policy_claims` as a JSON array of strings, as well as `consent=true`. Claims may be supplied in any order but must match the configured set exactly. Missing, duplicate, unknown, malformed, or stale acceptance returns 400 before a file is staged or a PR is created. Duplicate multipart fields are rejected. A successful submission stores optional `policy_acceptance:{version,claims,accepted_at}`, with claims in configured order and `accepted_at` in Unix seconds. Old submissions deserialize without a receipt. Without a configured policy, omitted fields or an empty version with an empty claims array are accepted and no receipt is added. Queued submissions are checked again before PR creation; enabling or changing a policy requires a fresh submission for tasks whose receipt no longer matches.

## meme-web

| Public route | Internal target |
| --- | --- |
| `GET /api/gallery` | core `/v1/gallery` |
| `GET /api/images/{hash}` | core `/v1/images/{hash}` |
| `GET /api/me` | core `/v1/me` |
| `GET /api/avatar` | GitHub `/auth/avatar`; only the session cookie is forwarded, never a query or request body |
| `GET/POST /api/submissions` | core `/v1/submissions` |
| `/auth/github`, `/auth/callback`, `/auth/logout` | Matching GitHub routes |
| `POST /webhooks/github` | Matching GitHub route, preserving the raw request body |
| `GET /source` | Source and version information |

Unknown paths return 404 and unsupported methods return 405. The public proxy rejects arbitrary upstream URLs and does not forward browser Authorization, Host, or forwarded identity headers. Service credentials stay within the services.

Locale resources under `crates/meme-web/assets/locales/` centralize English, Simplified Chinese, Japanese, Korean, and Russian text. The browser's language preferences select the interface language; unsupported languages and missing translation keys fall back to English. API fields and state values remain stable across languages.
