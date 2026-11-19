"""Writes THIRD-PARTY-NOTICES.txt for the Windows package: every crate built
into the program for Windows (normal dependencies only, not build or dev
dependencies), with its license expression and the license files it ships.

Usage: python tools/third_party_notices.py OUTPUT_FILE
Needs `cargo` and the crates' sources (after a build).
"""
import json
import os
import re
import subprocess
import sys

TARGET = "x86_64-pc-windows-msvc"
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice|copyright|ofl|ufl|unlicense)", re.I)


def metadata():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def shipped(meta):
    """Package ids reachable from the root through normal dependencies."""
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = meta["resolve"]["root"]
    seen, stack = set(), [root]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    seen.discard(root)
    return seen


def license_files(manifest_path):
    folder = os.path.dirname(manifest_path)
    found = []
    for base, dirs, files in os.walk(folder):
        depth = os.path.relpath(base, folder).count(os.sep)
        if depth > 1 or os.path.basename(base) in ("src", "tests", "benches", "examples", "target"):
            dirs[:] = []
            continue
        for name in sorted(files):
            if LICENSE_FILE.match(name) and not name.endswith((".rs", ".toml")):
                found.append(os.path.join(base, name))
    return sorted(found)


def main():
    meta = metadata()
    ids = shipped(meta)
    packages = sorted(
        (p for p in meta["packages"] if p["id"] in ids),
        key=lambda p: (p["name"].lower(), p["version"]),
    )
    out = open(sys.argv[1], "w", encoding="utf-8", newline="\n") if len(sys.argv) > 1 else sys.stdout
    out.write("NoirView includes the following third-party software.\n")
    out.write("Each entry gives the crate, its license and the license texts it ships.\n\n")
    for p in packages:
        out.write("=" * 78 + "\n")
        out.write(f"{p['name']} {p['version']}\n")
        out.write(f"License: {p.get('license') or 'see the license file below'}\n")
        if p.get("repository"):
            out.write(f"Source: {p['repository']}\n")
        if p.get("authors"):
            out.write("Authors: " + ", ".join(p["authors"]) + "\n")
        files = license_files(p["manifest_path"])
        if not files:
            out.write("(No license file in the published crate: the license named above applies.)\n")
        for path in files:
            out.write("-" * 78 + "\n")
            out.write(os.path.basename(path) + "\n\n")
            with open(path, encoding="utf-8", errors="replace") as f:
                out.write(f.read().rstrip() + "\n")
        out.write("\n")
    # The common licenses in full, for the crates that name them without
    # shipping a copy.
    here = os.path.dirname(os.path.abspath(__file__))
    eframe = os.path.join(here, "..", "vendor", "eframe")
    for title, name in (("MIT License (template)", "LICENSE-MIT"), ("Apache License 2.0", "LICENSE-APACHE")):
        out.write("=" * 78 + "\n")
        out.write(title + "\n\n")
        with open(os.path.join(eframe, name), encoding="utf-8") as f:
            text = f.read()
        if name == "LICENSE-MIT":
            text = "Copyright (c) <year> <copyright holders>\n" + text.split("\n", 1)[1]
        out.write(text.rstrip() + "\n\n")


if __name__ == "__main__":
    main()
