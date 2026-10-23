# External frontend crates

The default `cargo build` keeps the built-in frontend. To select an external frontend explicitly, use Python 3.11+ and the normal Rust 1.95 toolchain:

```sh
python scripts/frontend.py configure --path ../pig-memehub --crate crates/pig-memehub-web --package pig-memehub-web
python scripts/frontend.py build --offline
./target/debug/memehub --config config/local.toml serve
```

`configure` creates `frontend.local.toml` and refuses to overwrite an existing file. Local repository paths are resolved relative to the configuration file. `--crate` selects a member path within that repository; it defaults to `.`. `--feature NAME` is repeatable and `--no-default-features` is supported.

An equivalent configuration is:

```toml
[frontend]
package = "pig-memehub-web"
path = "../pig-memehub"
crate = "crates/pig-memehub-web"
features = []
default-features = true
```

A Git source requires an HTTPS URL and a full pinned commit, without credentials, query strings or fragments:

```sh
python scripts/frontend.py configure --config .local/selected-frontend.toml \
  --git https://github.com/OWNER/FRONTEND --rev FULL_40_CHARACTER_COMMIT \
  --package frontend-web
python scripts/frontend.py build --config .local/selected-frontend.toml --release
```

Cargo selects Git workspace packages by name, so `--crate` is only accepted for local sources. Only configure a repository you trust: compiling Rust dependencies also runs their build scripts and procedural macros. No remote frontend is discovered or loaded from runtime application settings.

## Router contract

The selected crate must expose:

```rust
pub fn router() -> axum::Router {
    axum::Router::new().route("/", axum::routing::get(|| async { "Hello" }))
}
```

Use Axum 0.8 with the same compatible dependency graph. The router owns pages and static assets. `/api`, `/auth`, `/webhooks`, and `/health` are reserved for the common web service. It must not add a wildcard that treats unknown private/API routes as pages. The service supplies security headers to pages, assets and API responses; it does not expose its secret configuration or upstream clients to the frontend.

The CSP allows scripts/styles/fonts from the same origin, images from the same origin and blob previews, and same-origin fetches. Use external JavaScript/CSS files and safe DOM operations; do not require inline-script or inline-style exceptions. Supply useful initial HTML and an explicit static-resource allowlist.

The selected frontend uses the existing public HTTP API. It does not replace session/CSRF/Origin validation, Webhook verification, upload limits, repository validation or publication. Existing runtime configuration, credentials and encrypted data work with the selected binary.

For custom launchers, the root library exposes `memehub::run_with_frontend(factory)`. `meme_web::run_with_frontend` serves an already-built router independently. The ordinary launcher calls `memehub::run()`.

## Generated runner and lockfile

The command creates a separate runner below `.local/frontend-build/`, seeded from the project's lockfile. It never edits either source repository's `Cargo.toml`. `inspect` resolves the dependency and prints the runner, lockfile and selected crate paths; `check` runs Cargo check. `--offline` forbids Cargo downloads. After the first resolution, use `--locked` to require the generated lockfile without updates:

```sh
python scripts/frontend.py inspect --offline
python scripts/frontend.py check --offline --locked
python scripts/frontend.py build --release --offline --locked
```

Keep the generated lockfile with private release records when reproducing a deployment. `--lockfile PATH` seeds a new runner from a previously saved combined lockfile. `--target` and `--target-dir` pass explicit compilation settings to Cargo. Switching between default and selected builds can replace the binary in the target directory; select a separate target directory when retaining both.

## Local image build

Commit the intended application and frontend files first, then run Docker on the build machine:

```sh
python scripts/frontend.py docker --tag pig-memehub:release --offline
```

Both source repositories must be Git checkouts with clean tracked files. Packaging copies only tracked regular files, refuses symlinks/submodules and path escapes, and bounds each repository to 20,000 files and 256 MiB. It excludes private configuration, `.env` files, PEM/key files, databases, `.local`, `.codex`, caches and build output. Untracked assets are not packaged; add and commit maintained assets first.

`--offline` controls the host Cargo resolution. Docker's separate build environment can still download its base images, OS packages and the already-locked dependencies. The generated context fixes the resolved frontend as a local snapshot and includes a combined lockfile. `Dockerfile.frontend` copies the frontend's available license/notice materials into `/app/frontend`, with source revisions in `build-source.json` and image labels. Local `--path` workspaces must keep their required source dependencies within the two bundled repositories; arbitrary sibling path dependencies are not bundled.

The resulting image keeps the same binary name, command-line interface, non-root user and runtime paths. Replace only the image reference in the existing deployment. Back up the configuration/database and retain the matching database key before an upgrade. Keep the App credentials, installation, Webhook secret and persistent mounts in place; never run `admin init` over a configured deployment.
