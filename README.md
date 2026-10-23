# memehub

[简体中文](docs/README.zh-CN.md)

A meme gallery backed by a GitHub repository. Users sign in with a GitHub App and upload an image. The app opens a pull request with the uploader credited as a coauthor. Repository Actions validate the image, generate its manifest, and publish the gallery index after merge.

Images stay in GitHub. RNovModularDB stores identities, sessions, submission status, and gallery metadata. The web interface follows the visitor's browser language: English, Chinese, Japanese, Korean, or Russian, with English as the fallback.

## Run locally

Requires Rust 1.95 and Git.

```sh
cargo build --locked
cargo run --locked -- admin init
cargo run --locked -- admin config set github.app_id YOUR_APP_ID
cargo run --locked -- admin config set github.client_id YOUR_CLIENT_ID
cargo run --locked -- admin config set github.installation_id 123456
cargo run --locked -- admin config set github.allowed_repositories '["OWNER/memes"]'
cargo run --locked -- admin config set core.repository OWNER/memes
cargo run --locked -- admin secret github-app --file /path/to/app.pem
cargo run --locked -- admin secret github-client --file /path/to/client-secret.txt
cargo run --locked -- serve
```

Open `http://localhost:7410`. Initialization creates `config/local.toml` and separate credentials in `.local/secrets/`. It refuses to overwrite existing files. Back up the database key alongside database backups; the database cannot be decrypted without it.

## Configure the GitHub App

1. Set the OAuth callback to `https://YOUR_HOST/auth/callback` and the webhook URL to `https://YOUR_HOST/webhooks/github`.
2. Grant repository Contents and Pull requests read/write, Metadata read, and user email read access. Subscribe to `pull_request`, `push`, `workflow_run`, `installation`, and `github_app_authorization` events.
3. Install the App on the image repository. Configure its App ID, Client ID, installation ID, private key, and client secret. Use the generated `.local/secrets/webhook-secret` as the App webhook secret.
4. Set `github.public_base` and `core.public_base` to the same public HTTPS URL.
5. Copy [repository-template](repository-template/README.md) into the image repository, including its `.github` directory. Follow its instructions for publishing credentials and branch protection.

Login requires a verified GitHub email. Uploads require consent to public review and coauthor attribution. The gallery stays empty until the first valid index is published. A failed refresh keeps the previous publication available.

## Deploy with Docker

```sh
export SOURCE_REVISION=$(git rev-parse HEAD)
docker compose build
docker compose run --rm app admin init
docker compose run --rm app admin config set web.listen 0.0.0.0:7410
# Configure the App with the admin commands above.
# Import a private key through stdin:
cat /path/to/app.pem | docker compose run --rm -T app admin secret github-app
docker compose up -d
```

Configuration and data use named volumes. The container runs as a non-root user with a read-only root filesystem. Service configuration changes require a restart:

```sh
docker compose restart app
docker compose logs --tail 100 app
```

Put an HTTPS reverse proxy on the same server, forwarding to `http://127.0.0.1:7410`. Preserve the original Host and Origin headers and allow request bodies of at least 6 MiB.

| Port | Exposure |
| --- | --- |
| TCP 443 | Public HTTPS: gallery, login callback, and webhook |
| TCP 80 | Optional HTTP redirect and HTTP-01 certificate validation |
| TCP 7410 | Loopback only with the supplied Compose file; no public firewall rule |
| TCP 7411–7413 | Internal database, GitHub, and core APIs; no public firewall rule |

If outbound traffic is restricted, allow HTTPS to GitHub. Image builds also need access to the configured container and package registries. Keep SSH access limited to your administration addresses.

## Administration

Run these commands through `docker compose exec app` when using Docker:

```sh
memehub admin cache show
memehub admin cache set --max-bytes 134217728 --max-entries 5000 --ttl-seconds 900
memehub admin sync
memehub admin submissions
memehub admin pulls
memehub admin retry <submission-uuid>
memehub admin backup
```

Cache settings persist in the database and take effect immediately. Lower limits evict entries immediately. File configuration changes take effect after restart. `admin data` provides generic record management.

Supported uploads: PNG, JPEG, GIF, and WebP; default maximum 5 MiB and 16,777,216 pixels per frame. `Name` accepts 1–64 ASCII letters, digits, underscores, or hyphens. Display names allow up to 120 Unicode characters. Each web upload creates one entry and one pull request.

## Select an external frontend

The default build includes `meme-web`. An external Rust crate can explicitly supply the page and asset router while the common service retains the API, OAuth, Webhooks and security headers:

```sh
python scripts/frontend.py configure --path ../pig-memehub --crate crates/pig-memehub-web --package pig-memehub-web
python scripts/frontend.py build --offline
```

Local workspace members, pinned HTTPS Git sources, feature selection and local Docker packaging are covered in [External frontend crates](docs/frontends.md). Existing server configuration and credentials remain separate from the selected frontend.

## Development

The five business crates communicate through the [HTTP API](docs/api.md), without Cargo dependencies on one another. Use `serve --service database|github|core|web` to run a service separately; start the database first.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --locked
docker compose config --quiet
```

AHCL materials are maintained with AHCL Kit and `.ahclkitconfigs`:

```sh
ahcl config validate
ahcl project generate
ahcl project check
```

Licensed under AHCL 1.2. See [LICENSE](LICENSE).
