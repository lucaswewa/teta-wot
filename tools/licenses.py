"""Gather the licence texts of every crate compiled into the release binaries.

A binary contains code from each of its dependencies, whose licences require
their notices to travel with it (ADR-0062). This walks the dependency graph
of the given workspace packages for Windows x86_64, as Cargo resolves it:

- normal dependencies only (not dev- or build-dependencies);
- not into procedural macros, which run when compiling and aren't linked.

`cargo metadata` resolves features for the whole workspace, so the list may
include a few crates that only another member's features pull in: it errs
on the side of too many notices, never too few.

For each crate it writes the name, version, declared licence and source,
then every licence, notice or copyright file in the package. Standard
library only:

    python tools/licenses.py simulated-microscope windows-service > THIRD_PARTY_LICENSES.txt

A summary of the licences found goes to standard error. It exits with an
error if a crate declares no licence.
"""

import json
import subprocess
import sys
from collections import Counter
from pathlib import Path

TARGET = "x86_64-pc-windows-msvc"
LICENCE_FILES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT", "UNLICENSE")

HEADER = """\
Third-party licences of the wot-rs binaries
===========================================

wot-rs itself has no licence (see THIRD_PARTY_NOTICES.md). The binaries in
this package contain code from the crates below, each under its own licence,
whose text follows its entry. Material copied into the wot-rs sources is
listed in THIRD_PARTY_NOTICES.md.
"""


def is_proc_macro(package):
    return any("proc-macro" in target["kind"] for target in package["targets"])


def main(roots):
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET],
            capture_output=True,
            text=True,
            encoding="utf-8",
            check=True,
        ).stdout
    )
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    workspace = set(metadata["workspace_members"])
    by_name = {packages[i]["name"]: i for i in workspace}
    missing = [name for name in roots if name not in by_name]
    if missing:
        sys.exit(f"no workspace package named {', '.join(missing)}")

    linked = set()
    stack = [by_name[name] for name in roots]
    while stack:
        package_id = stack.pop()
        if package_id in linked:
            continue
        linked.add(package_id)
        for dependency in nodes[package_id]["deps"]:
            normal = any(kind["kind"] is None for kind in dependency["dep_kinds"])
            target = packages[dependency["pkg"]]
            if normal and not is_proc_macro(target):
                stack.append(dependency["pkg"])

    crates = sorted(
        (packages[i] for i in linked if i not in workspace),
        key=lambda p: (p["name"], p["version"]),
    )
    out = [HEADER]
    licences = Counter()
    undeclared = []
    for crate in crates:
        declared = crate.get("license") or (crate.get("license_file") and "see its licence file")
        if not declared:
            undeclared.append(crate["name"])
        licences[declared or "undeclared"] += 1
        out.append("=" * 78)
        out.append(f"{crate['name']} {crate['version']}")
        out.append(f"Licence: {declared or 'not declared'}")
        if crate.get("repository"):
            out.append(f"Source: {crate['repository']}")
        folder = Path(crate["manifest_path"]).parent
        files = sorted(
            f for f in folder.iterdir() if f.is_file() and f.name.upper().startswith(LICENCE_FILES)
        )
        if not files:
            out.append("(The package has no licence file: its declared licence applies.)")
        for file in files:
            out.append(f"--- {file.name}")
            out.append(file.read_text(encoding="utf-8", errors="replace").rstrip())
        out.append("")
    sys.stdout.reconfigure(encoding="utf-8")
    print("\n".join(out))
    print(f"{len(crates)} crates linked into {', '.join(roots)}:", file=sys.stderr)
    for licence, count in licences.most_common():
        print(f"  {count:4}  {licence}", file=sys.stderr)
    if undeclared:
        sys.exit(f"crates without a declared licence: {', '.join(undeclared)}")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    main(sys.argv[1:])
