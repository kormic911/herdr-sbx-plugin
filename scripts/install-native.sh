#!/bin/sh
# Install the native component matching this plugin checkout.
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
version=$(sed -n 's/^version = "\([0-9][0-9.]*\)"[[:space:]]*$/\1/p' "$root/herdr-plugin.toml")
[ -n "$version" ] || { echo 'Missing plugin version' >&2; exit 1; }
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) target=x86_64-unknown-linux-musl ;;
  Linux/aarch64) target=aarch64-unknown-linux-musl ;;
  Darwin/x86_64) target=x86_64-apple-darwin ;;
  Darwin/arm64) target=aarch64-apple-darwin ;;
  *) echo 'No native release for this platform' >&2; exit 1 ;;
esac
asset="herdr-sbx-v$version-$target"
url="https://github.com/kormic911/herdr-sbx-plugin/releases/download/v$version/$asset"
dest="$root/target/release"
mkdir -p "$dest"
stage=$(mktemp -d "$dest/.download.XXXXXXXX")
trap 'rm -rf "$stage"' EXIT
curl -fsSL --retry 3 "$url" -o "$stage/binary"
curl -fsSL --retry 3 "$url.sha256" -o "$stage/hash"
if command -v sha256sum >/dev/null; then
  actual=$(sha256sum "$stage/binary")
else
  actual=$(shasum -a 256 "$stage/binary")
fi
[ "${actual%% *}" = "$(cat "$stage/hash")" ] || { echo 'Checksum mismatch' >&2; exit 1; }
chmod 755 "$stage/binary"
mv -f "$stage/binary" "$dest/herdr-sbx"
