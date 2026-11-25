#!/usr/bin/env python3
"""Generate license notices from platform-filtered Cargo metadata on stdin."""

import json
import re
import sys
from pathlib import Path

SUPPLEMENTAL = Path(__file__).with_name("licenses")
NOTICE_NAME = re.compile(
    r"^(licen[cs]e|copying|copyright|notice|third[-_]party[-_]notices?)([-_. ]|$)",
    re.IGNORECASE,
)
CRATES_IO = {
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
}


def release_dependencies(metadata):
    workspace = set(metadata["workspace_members"])
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root, = (package_id for package_id in workspace if packages[package_id]["name"] == "shardline")

    pending = [root]
    seen = set()
    while pending:
        package_id = pending.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        pending.extend(
            dependency["pkg"]
            for dependency in nodes[package_id]["deps"]
            if any(kind["kind"] is None for kind in dependency["dep_kinds"])
        )

    return sorted(
        (packages[package_id] for package_id in seen - workspace),
        key=lambda package: (package["name"], package["version"], package["id"]),
    )


def source_url(package):
    source = package["source"]
    if source in CRATES_IO:
        name = package["name"]
        version = package["version"]
        return f"https://static.crates.io/crates/{name}/{name}-{version}.crate"
    if source and source.startswith("git+"):
        repository, _, revision = source.removeprefix("git+").rpartition("#")
        repository = repository.split("?", 1)[0]
        if repository.startswith("https://github.com/"):
            return f"{repository.removesuffix('.git')}/tree/{revision}"
    return source or package["repository"] or "See Cargo.lock and the source checkout."


def notice_files(package, supplemental):
    root = Path(package["manifest_path"]).parent
    paths = {
        path
        for path in root.rglob("*")
        if path.is_file()
        and (NOTICE_NAME.match(path.name) or "LICENSES" in path.relative_to(root).parts[:-1])
    }
    if package["license_file"]:
        paths.add(root / package["license_file"])

    if paths:
        return [(str(path.relative_to(root)), path) for path in sorted(paths)]

    vcs = root / ".cargo_vcs_info.json"
    revision = json.loads(vcs.read_bytes()).get("git", {}).get("sha1") if vcs.is_file() else None
    if revision in supplemental:
        return [(url, SUPPLEMENTAL / file) for file, url in supplemental[revision].items()]

    raise SystemExit(
        f"No license text for {package['name']} {package['version']}. "
        "Check the published source and record any missing upstream license in scripts/licenses/."
    )


def main():
    packages = release_dependencies(json.load(sys.stdin))
    supplemental = json.loads((SUPPLEMENTAL / "upstream.json").read_bytes())
    texts = {}

    print("Shardline third-party notices")
    print("Cargo dependencies resolved through normal dependency edges for the release target.")
    print("The source link for each component identifies its corresponding source release.")
    print("For MPL-2.0 components, the corresponding Source Code Form is available from that link.")

    for package in packages:
        print(f"\n{package['name']} {package['version']}")
        print(f"  License: {package['license'] or 'See the notices below'}")
        print(f"  Source: {source_url(package)}")
        for name, path in notice_files(package, supplemental):
            content = path.read_text(encoding="utf-8", errors="surrogateescape")
            number = texts.setdefault(content, len(texts) + 1)
            print(f"  Notice {number}: {name}")

    for content, number in texts.items():
        print(f"\n--- Notice {number} ---")
        print(content, end="" if content.endswith("\n") else "\n")


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8", errors="surrogateescape")
    main()
