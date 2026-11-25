#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "$0")/.."
target=x86_64-unknown-linux-gnu
app=${1:-target/$target/release/literature-reader}
if [[ $(uname -s) != Linux || $(uname -m) != x86_64 || ! -x $app ]]; then
    echo 'Build the Linux x86_64 reader before packaging it.' >&2
    exit 1
fi

version=$(cargo metadata --locked --no-deps --format-version 1 | node -e '
    const metadata = JSON.parse(require("node:fs").readFileSync(0, "utf8"));
    console.log(metadata.packages.find(pkg => pkg.name === "literature-reader").version);
')
if [[ ! $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
    echo 'Cargo.toml must use a stable three-part version.' >&2
    exit 1
fi

mkdir -p target/release
staging=$(mktemp -d target/wayland-package.XXXXXXXX)
trap 'rm -rf -- "$staging"' EXIT
bundle=$staging/literature-reader
app_id=io.github.zhhpan.LiteratureReader
mkdir -p "$bundle/bridge" "$bundle/licenses/latin-modern" "$bundle/assets"
install -m 755 -- "$app" "$bundle/literature-reader"
cp -- bridge/{bridge.mjs,package.json,package-lock.json} "$bundle/bridge/"
npm ci --prefix "$bundle/bridge" --omit=dev --ignore-scripts --no-audit --no-fund --no-update-notifier

# The SDK selects its native helpers by Node.js platform and architecture.
vendor=$bundle/bridge/node_modules/@tencent-ai/agent-sdk/cli/vendor
find "$vendor/ripgrep" -mindepth 1 -maxdepth 1 -type d ! -name x64-linux -exec rm -r -- {} +
find "$vendor/genie-trash" -mindepth 1 -maxdepth 1 -type f \
    \( -name 'darwin-*' -o -name 'win32-*' -o -name linux-arm64 \) -delete
sandbox_kit=$vendor/sandbox-center-runtime-kit
rm -rf -- "$sandbox_kit/toybox-macos" "$sandbox_kit/zsh-macos" \
    "$sandbox_kit/shim/native/darwin-arm64" "$sandbox_kit/shim/native/darwin-x64"
rm -f -- "$sandbox_kit/shim/native/universal/broker-cli"
chmod +x -- "$vendor/ripgrep/x64-linux/rg" "$vendor/genie-trash/linux-x64"

# SHA-256 published in the signed v22.23.5 release checksums at nodejs.org.
node_dist=node-v22.23.5-linux-x64
node_sha256=cdeb8670a0c81f51b4dc4ac61919f9b1ad917b304bf8221eb939570663d240f8
node_archive=$staging/$node_dist.tar.xz
curl --fail --silent --show-error --location --retry 3 --connect-timeout 15 \
    --max-time 180 --output "$node_archive" "https://nodejs.org/dist/v22.23.5/$node_dist.tar.xz"
printf '%s  %s\n' "$node_sha256" "$node_archive" | sha256sum --check --status
tar -xJf "$node_archive" -C "$staging" -- "$node_dist/bin/node" "$node_dist/LICENSE"
install -Dm755 -- "$staging/$node_dist/bin/node" "$bundle/runtime/node/node"
install -m 644 -- "$staging/$node_dist/LICENSE" "$bundle/licenses/nodejs.txt"

install -m 644 -- LICENSE "$bundle/licenses/literature-reader.txt"
cp -- src/data/SCOWL-Copyright.txt "$bundle/licenses/"
cp -- src/data/latex-fonts/{*.txt,README.md} "$bundle/licenses/latin-modern/"
mkdir -p "$bundle/share/fonts/literature-reader"
cp -- src/data/latex-fonts/*.otf "$bundle/share/fonts/literature-reader/"
pwsh -NoLogo -NoProfile -NonInteractive -File installer/collect-rust-notices.ps1 \
    -Target "$target" -OutputPath "$bundle/licenses/rust-dependencies.txt"
install -m 644 -- assets/reader.png "$bundle/assets/reader.png"
install -Dm644 -- assets/reader.png "$bundle/share/icons/hicolor/512x512/apps/$app_id.png"
install -Dm644 -- "installer/linux/$app_id.desktop" "$bundle/share/applications/$app_id.desktop"
install -m 644 -- installer/linux/README.md "$bundle/README.md"

archive=LiteratureReader-Wayland-$version-x86_64.tar.gz
tar --sort=name --owner=0 --group=0 --numeric-owner \
    -C "$staging" -czf "$staging/$archive" literature-reader
mv -f -- "$staging/$archive" "target/release/$archive"
printf 'Packaged target/release/%s\n' "$archive"
