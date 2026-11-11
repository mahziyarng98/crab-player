#!/usr/bin/env python3
"""Write cargo-sources.json for flatpak-builder from Cargo.lock.

flatpak-builder builds without network access, so every crate is listed as
a source: downloaded from crates.io, checked against the Cargo.lock
checksum and unpacked into cargo/vendor, where a cargo config points the
crates.io registry. Only registry crates are supported (Crab Player has no
git dependencies).

Usage: packaging/flatpak/cargo-sources.py [Cargo.lock] [output.json]
"""

import json
import os
import sys
import tomllib

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"


def main():
    lock_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "Cargo.lock")
    out_path = (
        sys.argv[2]
        if len(sys.argv) > 2
        else os.path.join(ROOT, "packaging", "flatpak", "cargo-sources.json")
    )
    with open(lock_path, "rb") as lock_file:
        lock = tomllib.load(lock_file)

    sources = []
    for package in lock["package"]:
        source = package.get("source")
        if source is None:
            continue  # the workspace itself
        if source != CRATES_IO:
            sys.exit(f"unsupported source for {package['name']}: {source}")
        name, version, checksum = package["name"], package["version"], package["checksum"]
        dest = f"cargo/vendor/{name}-{version}"
        sources.append(
            {
                "type": "archive",
                "archive-type": "tar-gzip",
                "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                "sha256": checksum,
                "dest": dest,
            }
        )
        # Cargo checks vendored crates against this file; the package hash
        # is what Cargo.lock records, the per-file list may be empty.
        sources.append(
            {
                "type": "inline",
                "contents": json.dumps({"package": checksum, "files": {}}),
                "dest": dest,
                "dest-filename": ".cargo-checksum.json",
            }
        )
    sources.append(
        {
            "type": "inline",
            "contents": (
                '[source.vendored-sources]\ndirectory = "cargo/vendor"\n\n'
                '[source.crates-io]\nreplace-with = "vendored-sources"\n'
            ),
            "dest": "cargo",
            "dest-filename": "config.toml",
        }
    )
    with open(out_path, "w", encoding="utf-8") as out:
        json.dump(sources, out, indent=1)
        out.write("\n")
    print(f"{len(sources) // 2} crates written to {os.path.relpath(out_path, ROOT)}")


if __name__ == "__main__":
    main()
