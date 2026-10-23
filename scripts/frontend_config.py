"""Explicit frontend configuration. This module never downloads repositories."""
from dataclasses import dataclass
import json
import os
from pathlib import Path, PurePosixPath
import re
import tomllib
from urllib.parse import urlsplit


@dataclass(frozen=True)
class Frontend:
    package: str
    path: Path | None = None
    git: str | None = None
    rev: str | None = None
    crate: str = "."
    features: tuple[str, ...] = ()
    default_features: bool = True


def require(condition, message):
    if not condition:
        raise ValueError(message)


def text(value, name):
    require(isinstance(value, str), f"{name} must be a string")
    require(0 < len(value) <= 2048, f"{name} is empty or too long")
    require(not any(ord(char) < 32 for char in value), f"{name} contains a control character")
    return value


def package_name(value):
    value = text(value, "frontend.package")
    require(re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]{0,63}", value), "Invalid Cargo package name")
    return value


def crate_path(value):
    value = text(value, "frontend.crate")
    path = PurePosixPath(value)
    require(not path.is_absolute(), "frontend.crate must be relative")
    require(".." not in path.parts, "frontend.crate cannot escape its repository")
    require("\\" not in value and ":" not in value, "frontend.crate must use relative forward-slash paths")
    return str(path)


def git_source(data):
    git = text(data.get("git"), "frontend.git")
    url = urlsplit(git)
    require(url.scheme == "https" and bool(url.hostname), "Git source must use HTTPS")
    require(url.username is None and url.password is None, "Git source must not contain credentials")
    require(not url.query and not url.fragment, "Git source must not contain a query or fragment")
    rev = text(data.get("rev"), "frontend.rev")
    require(re.fullmatch(r"[0-9a-f]{40}", rev), "Git source requires a full lowercase 40-character commit")
    require("crate" not in data, "Cargo selects a Git workspace package by name; omit frontend.crate")
    return git, rev


def local_source(data, base, package):
    value = text(data.get("path"), "frontend.path")
    root = (base / value).resolve(strict=True)
    require(root.is_dir(), "frontend.path must name a repository directory")
    member = crate_path(data.get("crate", "."))
    manifest = (root / member / "Cargo.toml").resolve(strict=True)
    require(manifest.is_relative_to(root), "Frontend crate escapes its repository")
    require(manifest.is_file(), "Frontend Cargo.toml is not a regular file")
    contents = tomllib.loads(manifest.read_text(encoding="utf-8"))
    require(contents.get("package", {}).get("name") == package, "Frontend package does not match Cargo.toml")
    return root, member


def feature_list(value):
    require(isinstance(value, list), "frontend.features must be an array")
    require(len(value) <= 32, "At most 32 frontend features are allowed")
    for item in value:
        require(isinstance(item, str), "Frontend features must be strings")
        require(re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_+-]{0,63}", item), "Invalid frontend feature")
    require(len(set(value)) == len(value), "Duplicate frontend feature")
    return tuple(value)


def parse(data, base):
    allowed = {"package", "path", "git", "rev", "crate", "features", "default-features"}
    require(isinstance(data, dict), "[frontend] must be a table")
    require(not set(data) - allowed, "Unknown [frontend] configuration field")
    require(("path" in data) != ("git" in data), "Set exactly one of frontend.path or frontend.git")
    name = package_name(data.get("package"))
    features = feature_list(data.get("features", []))
    default = data.get("default-features", True)
    require(isinstance(default, bool), "frontend.default-features must be boolean")
    if "git" in data:
        git, rev = git_source(data)
        return Frontend(name, git=git, rev=rev, features=features, default_features=default)
    require("rev" not in data, "frontend.rev is only valid with frontend.git")
    root, member = local_source(data, base, name)
    return Frontend(name, path=root, crate=member, features=features, default_features=default)


def load(path):
    require(path.is_file() and path.stat().st_size <= 16384, "Frontend config missing or exceeds 16 KiB")
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    require(set(data) == {"frontend"}, "Frontend configuration must contain only [frontend]")
    return parse(data["frontend"], path.parent)


def quote(value):
    return json.dumps(value, ensure_ascii=True)


def render(frontend, base):
    rows = ["[frontend]", f"package = {quote(frontend.package)}"]
    if frontend.path is not None:
        try:
            path = Path(os.path.relpath(frontend.path, base)).as_posix()
        except ValueError:
            path = frontend.path.as_posix()
        rows.extend([f"path = {quote(path)}", f"crate = {quote(frontend.crate)}"])
    else:
        rows.extend([f"git = {quote(frontend.git)}", f"rev = {quote(frontend.rev)}"])
    rows.extend([f"features = {json.dumps(frontend.features)}", f"default-features = {str(frontend.default_features).lower()}"])
    return "\n".join(rows) + "\n"
