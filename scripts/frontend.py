#!/usr/bin/env python3
"""Configure, build or package an explicitly selected frontend crate."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from frontend_config import load, parse, render, require
from frontend_docker import docker
from frontend_runner import build, metadata, prepare, selected_package

APP = Path(__file__).resolve().parent.parent


def parser():
    cli = argparse.ArgumentParser(description=__doc__)
    commands = cli.add_subparsers(dest="command", required=True)
    configure = commands.add_parser("configure", help="Write a new frontend configuration without overwriting")
    configure.add_argument("--config", type=Path, default=Path("frontend.local.toml"))
    source = configure.add_mutually_exclusive_group(required=True)
    source.add_argument("--path", type=Path, help="Local repository root")
    source.add_argument("--git", help="HTTPS Git repository URL")
    configure.add_argument("--package", required=True, help="Cargo package exposing router() -> axum::Router")
    configure.add_argument("--crate", help="Crate path relative to a local repository")
    configure.add_argument("--rev", help="Full pinned commit for a Git source")
    configure.add_argument("--feature", action="append", default=[])
    configure.add_argument("--no-default-features", action="store_true")
    for name in ("build", "check", "inspect", "docker"):
        command = commands.add_parser(name)
        command.add_argument("--config", type=Path, default=Path("frontend.local.toml"))
        command.add_argument("--offline", action="store_true")
        add_build_arguments(command, name)
    return cli


def add_build_arguments(command, name):
    if name in ("build", "check"):
        command.add_argument("--release", action="store_true")
        command.add_argument("--locked", action="store_true")
        command.add_argument("--target")
        command.add_argument("--target-dir", type=Path)
        command.add_argument("--lockfile", type=Path, help="Seed lockfile for a new generated runner")
    if name == "docker":
        command.add_argument("--tag", required=True)
        command.add_argument("--platform", choices=["linux/amd64", "linux/arm64"], default="linux/amd64")
        command.add_argument("--scratch-dir", type=Path, help="Temporary Docker context parent")


def configure(options):
    output = options.config.absolute()
    require(not output.exists(), "Frontend configuration already exists; choose another --config")
    data = {"package": options.package, "features": options.feature, "default-features": not options.no_default_features}
    for name in ("path", "git", "rev", "crate"):
        value = getattr(options, name)
        if value is not None:
            data[name] = str(value)
    frontend = parse(data, Path.cwd())
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("x", encoding="utf-8", newline="\n") as stream:
        stream.write(render(frontend, output.parent))
    print(f"Created {output}", flush=True)


def execute(options):
    if options.command == "configure":
        return configure(options)
    frontend = load(options.config.resolve())
    if options.command == "docker":
        return docker(frontend, APP, options)
    if options.command == "inspect":
        work = prepare(frontend, APP)
        data = metadata(work, options.offline)
        package = selected_package(data)
        print(json.dumps({"manifest": str(work / "Cargo.toml"), "lockfile": str(work / "Cargo.lock"), "frontend_manifest": package["manifest_path"], "package": package["name"]}, indent=2))
        return None
    return build(frontend, APP, options)


def main():
    try:
        execute(parser().parse_args())
    except (ValueError, OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Frontend build failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
