"""Generate a separate Cargo runner without changing either source manifest."""
import hashlib
import json
from pathlib import Path
import subprocess

from frontend_config import quote, require

MAIN = """#[tokio::main]
async fn main() -> anyhow::Result<()> {
    memehub::run_with_frontend(selected_frontend::router).await
}
"""


def dependency(frontend):
    rows = ["[dependencies.selected-frontend]", f"package = {quote(frontend.package)}"]
    if frontend.path is not None:
        rows.append(f"path = {quote((frontend.path / frontend.crate).as_posix())}")
    else:
        rows.extend([f"git = {quote(frontend.git)}", f"rev = {quote(frontend.rev)}"])
    rows.extend([f"features = {json.dumps(frontend.features)}", f"default-features = {str(frontend.default_features).lower()}"])
    return "\n".join(rows)


def manifest(frontend, app):
    return f'''[workspace]
resolver = "3"

[package]
name = "memehub-frontend-runner"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
publish = false

[[bin]]
name = "memehub"
path = "src/main.rs"

[dependencies]
memehub = {{ path = {quote(app.as_posix())} }}
anyhow = "1"
tokio = {{ version = "1", features = ["macros", "rt-multi-thread"] }}

{dependency(frontend)}

[profile.release]
strip = true
lto = "thin"
'''


def write_generated(path, content):
    if path.exists():
        require(path.is_file() and not path.is_symlink(), f"Invalid generated path: {path}")
        require(path.read_text(encoding="utf-8") == content, f"Generated file changed unexpectedly: {path}")
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8", newline="\n") as stream:
        stream.write(content)


def prepare(frontend, app, lockfile=None):
    content = manifest(frontend, app)
    identity = hashlib.sha256(content.encode()).hexdigest()[:20]
    work = app / ".local" / "frontend-build" / identity
    require(work.resolve().is_relative_to(app.resolve()), "Generated build directory escapes application")
    directories = (app / ".local", app / ".local" / "frontend-build", work, work / "src")
    for directory in directories:
        require(not directory.is_symlink(), "Generated build directory must not be a symlink")
        require(not getattr(directory, "is_junction", lambda: False)(), "Generated build directory must not be a junction")
    write_generated(work / "Cargo.toml", content)
    write_generated(work / "src" / "main.rs", MAIN)
    lock = work / "Cargo.lock"
    require(not lock.is_symlink(), "Generated lockfile must not be a symlink")
    if lock.exists():
        require(lock.is_file(), "Generated lockfile must be a regular file")
    else:
        seed = lockfile or app / "Cargo.lock"
        require(seed.is_file(), "A seed Cargo.lock is required")
        with lock.open("xb") as stream:
            stream.write(seed.read_bytes())
    return work


def cargo_args(command, work, options):
    args = ["cargo", command, "--manifest-path", str(work / "Cargo.toml")]
    for name in ("offline", "locked", "release"):
        if getattr(options, name, False):
            args.append("--" + name)
    if getattr(options, "target", None):
        args.extend(["--target", options.target])
    if getattr(options, "target_dir", None):
        args.extend(["--target-dir", str(options.target_dir.resolve())])
    return args


def build(frontend, app, options):
    work = prepare(frontend, app, options.lockfile)
    command = "check" if options.command == "check" else "build"
    args = cargo_args(command, work, options)
    if not options.target_dir:
        args.extend(["--target-dir", str(app / "target")])
    subprocess.run(args, check=True)
    print(f"Frontend runner: {work / 'Cargo.toml'}", flush=True)
    return work


def metadata(work, offline=False):
    args = ["cargo", "metadata", "--manifest-path", str(work / "Cargo.toml"), "--format-version", "1"]
    if offline:
        args.append("--offline")
    result = subprocess.run(args, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(result.stderr.strip())
    return json.loads(result.stdout)


def selected_package(data):
    root = data["resolve"]["root"]
    node = next(row for row in data["resolve"]["nodes"] if row["id"] == root)
    selected = next(row["pkg"] for row in node["deps"] if row["name"] == "selected_frontend")
    return next(row for row in data["packages"] if row["id"] == selected)
