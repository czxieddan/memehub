"""Create a bounded Docker context from explicit, tracked repositories."""
from dataclasses import replace
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tempfile

from frontend_config import render, require
from frontend_runner import metadata, prepare, selected_package

EXCLUDED = {".git", ".local", ".codex", "target", "__pycache__", ".pytest_cache", "node_modules"}
PRIVATE_NAMES = {".env", "local.toml", "frontend.local.toml"}
PRIVATE_SUFFIXES = {".pem", ".key", ".rnmdb", ".p12", ".pfx"}


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def repository_root(path):
    return Path(git(path, "rev-parse", "--show-toplevel")).resolve()


def allowed_file(name):
    path = PurePosixPath(name)
    require(not path.is_absolute() and ".." not in path.parts, "Invalid tracked path")
    require("\\" not in name and ":" not in name, "Invalid tracked filename")
    if EXCLUDED.intersection(path.parts):
        return False
    if path.name in PRIVATE_NAMES or path.name.startswith(".env."):
        return False
    return path.suffix.lower() not in PRIVATE_SUFFIXES


def regular_source(root, name):
    source = root / name
    require(not source.is_symlink(), f"Symlink is not allowed in build context: {name}")
    resolved = source.resolve(strict=True)
    require(resolved.is_relative_to(root), f"Tracked path escapes repository: {name}")
    require(resolved.is_file(), f"Tracked build input is not a regular file: {name}")
    for parent in source.parents:
        if parent == root:
            break
        require(not parent.is_symlink(), f"Symlink parent in build input: {name}")
    return resolved


def copy_tracked(root, destination):
    require(repository_root(root) == root, "Docker inputs must name repository roots")
    require(not git(root, "status", "--porcelain", "--untracked-files=no"), "Commit tracked changes before Docker packaging")
    names = subprocess.check_output(["git", "-C", str(root), "ls-files", "-z", "--cached"]).decode("utf-8").split("\0")
    selected = [name for name in names if name and allowed_file(name)]
    require(len(selected) <= 20000, "Build context exceeds 20000 files")
    sources = [(name, regular_source(root, name)) for name in selected]
    total = sum(source.stat().st_size for _, source in sources)
    require(total <= 256 * 1024 * 1024, "Build context exceeds 256 MiB")
    for name, source in sources:
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    require((destination / "Cargo.toml").is_file(), "Build context has no tracked Cargo.toml")
    return {"files": len(sources), "bytes": total, "revision": git(root, "rev-parse", "HEAD")}


def copy_materials(frontend_root, destination):
    destination.mkdir()
    for name in ("LICENSE", "LICENSE.md", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "NOTICE.md", ".ahcl"):
        source = frontend_root / name
        if not source.exists():
            continue
        if source.is_dir():
            shutil.copytree(source, destination / name)
        else:
            shutil.copyfile(source, destination / name)


def context(frontend, app, destination, offline):
    work = prepare(frontend, app)
    package = selected_package(metadata(work, offline))
    member = Path(package["manifest_path"]).resolve().parent
    frontend_root = repository_root(member)
    app_info = copy_tracked(app, destination / "memehub")
    frontend_info = copy_tracked(frontend_root, destination / "frontend")
    bundled = replace(frontend, path=destination / "frontend", git=None, rev=None, crate=member.relative_to(frontend_root).as_posix())
    (destination / "frontend.toml").write_text(render(bundled, destination), encoding="utf-8")
    shutil.copyfile(work / "Cargo.lock", destination / "runner.lock")
    shutil.copyfile(app / "Dockerfile.frontend", destination / "Dockerfile")
    copy_materials(destination / "frontend", destination / "frontend-materials")
    proof = {"application": app_info, "frontend": frontend_info, "package": frontend.package, "git": frontend.git, "rev": frontend.rev}
    (destination / "frontend-materials" / "build-source.json").write_text(json.dumps(proof, indent=2) + "\n")
    return proof


def docker(frontend, app, options):
    require(re.fullmatch(r"[a-z0-9][a-z0-9./:_-]{0,180}", options.tag), "Invalid Docker image tag")
    with tempfile.TemporaryDirectory(prefix="memehub-frontend-", dir=options.scratch_dir) as folder:
        destination = Path(folder).resolve()
        proof = context(frontend, app, destination, options.offline)
        args = ["docker", "build", "--platform", options.platform, "--progress=plain", "-t", options.tag]
        args.extend(["--build-arg", "SOURCE_REVISION=" + proof["application"]["revision"]])
        args.extend(["--build-arg", "FRONTEND_REVISION=" + proof["frontend"]["revision"]])
        args.append(str(destination))
        subprocess.run(args, check=True)
        print(json.dumps(proof, indent=2), flush=True)
