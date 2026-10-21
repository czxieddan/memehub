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
| `POST /webhooks/github` | Verifies `X-Hub-Signature-256` over the raw body and deduplicates by delivery ID |
| `GET /v1/session` | `{id,login,email,contribution_email,csrf_token}`; 401 for an invalid session |
| `POST /v1/changes` | Creates an App branch and PR |
| `GET /v1/pulls/{number}?repository=OWNER/REPO` | PR status, including `state`, `merged`, and `url` |
| `GET /v1/pulls?repository=OWNER/REPO&page=1` | `{items,next_page}` for reconciliation |
| `GET /v1/content?repository=&path=&reference=` | `{content_base64}`; 404 when absent |
| `GET /v1/events?cursor=` | Persisted events as `{items:[{cursor,event}],next_cursor}` |

A change request contains `operation_id` (a UUID idempotency key), `repository`, `base_branch`, `title`, `body`, `message`, optional `coauthor_user_id`, and `files:[{path,content_base64}]`. Configuration restricts repositories and path prefixes. Attribution comes from the saved authorized identity. The response is `{operation_id,repository,number,url,branch}`. Base64 is used for transport and is not stored in the database.

## meme-core

| Method and path | Contract |
| --- | --- |
| `GET /v1/gallery?query=&cursor=&limit=` | `{items:[{manifest_hash,uploader_id,name,display_name}],next_cursor}`; default page size 40 |
| `GET /v1/images/{hash}` | Published image with the correct Content-Type and nosniff |
| `GET /v1/me` | Session identity; 401 for an invalid session |
| `GET /v1/submissions` | Current user's submission statuses |
| `POST /v1/submissions` | Multipart `name`, `display_name`, `file`, `consent=true`; requires a valid cookie, Origin, and `X-CSRF-Token` |
| `GET /v1/admin/cache` | Policy, usage, and statistics |
| `PUT /v1/admin/cache` | Persists and immediately applies `{max_bytes,max_entries,ttl_seconds}` |
| `POST /v1/admin/sync` | Fetches and validates the repository index, then switches publication transactionally |
| `GET /v1/admin/submissions` | Submission tasks |
| `GET /v1/admin/pulls` | Reconciled web and external PRs |
| `POST /v1/admin/submissions/{id}/retry` | Retries a failed PR creation task |

All `/v1/admin/*` routes require the core administrator token. Submission states progress through `pending`, `retrying`, `pr_open`, `awaiting_publish`, and `published`, or enter `failed` or `closed`. `merged=true` alone does not indicate publication: the valid index must contain the entry.

## meme-web

| Public route | Internal target |
| --- | --- |
| `GET /api/gallery` | core `/v1/gallery` |
| `GET /api/images/{hash}` | core `/v1/images/{hash}` |
| `GET /api/me` | core `/v1/me` |
| `GET/POST /api/submissions` | core `/v1/submissions` |
| `/auth/github`, `/auth/callback`, `/auth/logout` | Matching GitHub routes |
| `POST /webhooks/github` | Matching GitHub route, preserving the raw request body |
| `GET /source` | Source and version information |

Unknown paths return 404 and unsupported methods return 405. The public proxy rejects arbitrary upstream URLs and does not forward browser Authorization, Host, or forwarded identity headers. Service credentials stay within the services.

Locale resources under `crates/meme-web/assets/locales/` centralize English, Simplified Chinese, Japanese, Korean, and Russian text. The browser's language preferences select the interface language; unsupported languages and missing translation keys fall back to English. API fields and state values remain stable across languages.
