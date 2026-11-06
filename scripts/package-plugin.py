"""Build one plugin project and package it into an installable plugin folder.

A Brucebox plugin is a folder with a manifest and a platform-specific native
library (SPEC §5.2). This script does the two steps a developer needs:

1. build *that one* plugin project with Cargo (`cargo build` in its own project,
   never as part of the host's build), and
2. copy the manifest and the built library into the package layout the loader
   reads, under the host's `target/plugin-packages/<plugin-id>/`.

It never touches the host build and never runs as part of normal application
startup: it is a development tool.

Usage: python3 scripts/package-plugin.py [PROJECT] [--release]

    PROJECT   plugin project directory (default: plugins/agent-chat)
"""

from __future__ import annotations

import argparse
import pathlib
import shutil
import subprocess
import sys

import tomllib

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
PACKAGES_DIR = REPO_ROOT / "target" / "plugin-packages"


def platform_key() -> str:
    """The manifest key for this machine, matching what the loader looks up."""
    system = sys.platform
    machine = platform_machine()
    if system == "darwin":
        return f"macos-{machine}"
    if system == "win32":
        return f"windows-{machine}"
    return f"linux-{machine}"


def platform_machine() -> str:
    machine = subprocess.run(
        ["uname", "-m"], capture_output=True, text=True, check=True
    ).stdout.strip()
    return {"arm64": "aarch64", "AMD64": "x86_64"}.get(machine, machine)


def built_library(project: pathlib.Path, release: bool) -> pathlib.Path:
    profile = "release" if release else "debug"
    target = project / "target" / profile
    patterns = ["*.so", "*.dylib", "*.dll"]
    candidates = sorted(path for pattern in patterns for path in target.glob(pattern))
    if not candidates:
        raise SystemExit(f"no plugin library found in {target}")
    # A cdylib project produces exactly one; anything else here is a mistake
    # worth reporting rather than guessing about.
    if len(candidates) > 1:
        names = ", ".join(candidate.name for candidate in candidates)
        raise SystemExit(f"expected one plugin library in {target}, found: {names}")
    return candidates[0]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project", nargs="?", default="plugins/agent-chat")
    parser.add_argument("--release", action="store_true")
    args = parser.parse_args()

    project = (REPO_ROOT / args.project).resolve()
    manifest_path = project / "plugin.toml"
    if not manifest_path.is_file():
        raise SystemExit(f"{manifest_path} is missing: not a plugin project")

    manifest = tomllib.loads(manifest_path.read_text())
    plugin_id = manifest["plugin"]["id"]
    declared = manifest.get("library", {})
    key = platform_key()
    if key not in declared:
        raise SystemExit(f"plugin.toml declares no library for `{key}`")

    build = ["cargo", "build", "--manifest-path", str(project / "Cargo.toml")]
    if args.release:
        build.append("--release")
    print(f"building {plugin_id} …")
    subprocess.run(build, check=True, cwd=project)

    library = built_library(project, args.release)

    package = PACKAGES_DIR / plugin_id
    if package.exists():
        try:
            shutil.rmtree(package)
        except OSError as error:
            raise SystemExit(
                f"cannot replace the old package at {package}: {error}"
            ) from error

    relative = pathlib.Path(declared[key])
    destination = package / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(library, destination)
    shutil.copy2(manifest_path, package / "plugin.toml")

    print(f"packaged {plugin_id}")
    print(f"  folder:  {package}")
    print(f"  library: {relative}")
    print()
    print("install it by copying that folder into a plugin root, for example:")
    print(f"  BRUCEBOX_PLUGIN_DIR={PACKAGES_DIR} cargo run")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
