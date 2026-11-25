#!/usr/bin/env bash
set -euo pipefail

archive=$(realpath -- "${1:?Usage: smoke-wayland.sh path/to/package.tar.gz}")
staging=$(mktemp -d)
weston_pid=
cleanup() {
    if [[ -n $weston_pid ]]; then
        kill "$weston_pid" 2>/dev/null || true
        wait "$weston_pid" 2>/dev/null || true
    fi
    rm -rf -- "$staging"
}
trap cleanup EXIT

tar -xzf "$archive" -C "$staging"
bundle=$staging/literature-reader
app_id=io.github.zhhpan.LiteratureReader
[[ -x $bundle/literature-reader && -x $bundle/runtime/node/node ]]
[[ $("$bundle/runtime/node/node" --version) == v22.23.5 ]]
"$bundle/runtime/node/node" --check "$bundle/bridge/bridge.mjs"
(
    cd -- "$bundle/bridge"
    ../runtime/node/node --input-type=module -e '
        const sdk = await import("@tencent-ai/agent-sdk");
        if (typeof sdk.unstable_v2_createSession !== "function") process.exit(1);
    '
)
desktop-file-validate "$bundle/share/applications/$app_id.desktop"
pdftotext -h >"$staging/poppler-help.log" 2>&1 || true
if ! grep -q -- '-tsv' "$staging/poppler-help.log"; then
    echo 'Poppler pdftotext with TSV support is required.' >&2
    exit 1
fi

export XDG_RUNTIME_DIR=$staging/run
export WAYLAND_DISPLAY=literature-reader-ci
export GDK_BACKEND=wayland
export GSK_RENDERER=cairo
unset DISPLAY
mkdir -m 700 -- "$XDG_RUNTIME_DIR"
weston --backend=headless --shell=kiosk-shell.so --renderer=pixman \
    --socket="$WAYLAND_DISPLAY" --no-config --idle-time=0 --log="$staging/weston.log" &
weston_pid=$!
for _ in {1..100}; do
    [[ -S $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY ]] && break
    kill -0 "$weston_pid" 2>/dev/null || break
    sleep 0.1
done
if [[ ! -S $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY ]]; then
    cat "$staging/weston.log" >&2
    echo 'Headless Weston did not create its Wayland socket.' >&2
    exit 1
fi

if ! timeout --kill-after=3s 25s dbus-run-session -- "$bundle/literature-reader" --wayland-smoke \
    >"$staging/reader.log" 2>&1 || ! grep -Fxq WAYLAND_SMOKE_OK "$staging/reader.log"; then
    cat "$staging/weston.log" "$staging/reader.log" >&2
    exit 1
fi
cat "$staging/reader.log"
