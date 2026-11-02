#!/bin/bash
# Bundle what `npx dsh-mirsa setup` needs so it works without Homebrew or
# GitHub release downloads: the MIRSA source, z3 headers+libs, and a standalone
# Python >= 3.10 for the MCP venv.
set -e
here="$(cd "$(dirname "$0")/.." && pwd)"
root="$(cd "$here/.." && pwd)"
mkdir -p "$here/vendor"

tar -czf "$here/vendor/mirsa-src.tar.gz" -C "$root" \
  --exclude='target' --exclude='third-party' --exclude='backup' --exclude='.git' \
  --exclude='dsh-plugin/vendor' --exclude='mirsa-trace.jsonl' --exclude='last-run.json' \
  Cargo.toml Cargo.lock rust-toolchain.toml README.md UNSAFE_API_SUPPORT.md \
  crates src tools scripts docs tests examples

if [ -d "$root/third-party/z3/include" ] && [ -d "$root/third-party/z3/lib" ]; then
  tar -czf "$here/vendor/z3.tar.gz" -C "$root/third-party/z3" include lib
else
  echo "warn: no $root/third-party/z3, vendor/z3.tar.gz not refreshed" >&2
fi

if [ -x "$root/third-party/python/bin/python3" ]; then
  tar -czf "$here/vendor/python.tar.gz" -C "$root/third-party" python
else
  echo "warn: no $root/third-party/python, vendor/python.tar.gz not refreshed" >&2
fi

ls -lh "$here/vendor"
