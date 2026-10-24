#!/usr/bin/env bash
# Build the release archive the GitHub workflow publishes:
#   dist/helterm-<version>-<arch>-linux.tar.gz and a .sha256 next to it.
# Runs the same locally and in CI. With RELEASE_TAG set (CI sets it from the
# git tag), it refuses to build when the tag doesn't match Cargo.toml.
set -euo pipefail
cd "$(dirname "$0")/.."

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
if [ -n "${RELEASE_TAG:-}" ] && [ "$RELEASE_TAG" != "v$version" ]; then
    echo "error: tag $RELEASE_TAG does not match Cargo.toml version $version (expected v$version)" >&2
    exit 1
fi

cargo build --release --locked

name="helterm-$version-$(uname -m)-linux"
stage="dist/$name"
rm -rf "$stage" "dist/$name.tar.gz" "dist/$name.tar.gz.sha256"
mkdir -p "$stage"
install -m755 target/release/helterm "$stage/"
install -m755 packaging/install.sh "$stage/"
cp packaging/*.desktop packaging/*.svg config.example.toml README.md "$stage/"
if [ -f LICENSE ]; then cp LICENSE "$stage/"; fi

tar -C dist -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
rm -rf "$stage"
echo "dist/$name.tar.gz"
