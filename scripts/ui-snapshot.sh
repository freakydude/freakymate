#!/usr/bin/env bash
# Renders the pages of the Slint UI headless as PNG against a freshly started fake Moonraker.
# WSL2/Linux counterpart of ui-snapshot.ps1.
# Usage (repo root): scripts/ui-snapshot.sh [-o <folder>] [-p voron] [-s 800x480] [-F] [-S pages,network] [-P 7126]
# -P: port of its own fake; 7126 by default, so a fake on Moonraker's 7125 (the debug setup) is left alone.
# -F: no fit; the window of the given size without scaling the layout to 800x480 (as a desktop window; at least
#     800x480). Without -F the layout is fitted to the size like the app does in fullscreen.
# Without -S all scenes except the slow 'idle' and 'login' run; with 'login' the fake wants a login (user test,
# password secret) and the other chosen scenes run logged in. Exit code 0 on success; the PNG paths are printed.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
out_dir="${TMPDIR:-/tmp}/freakymate-ui-snapshots"
profile=voron
size=800x480
scenes=
port=7126
fit_flag=()

while getopts "o:p:s:S:P:Fh" option; do
    case $option in
        o) out_dir=$OPTARG ;;
        p) profile=$OPTARG ;;
        s) size=$OPTARG ;;
        S) scenes=$OPTARG ;;
        P) port=$OPTARG ;;
        F) fit_flag=(--no-fit) ;;
        *) sed -n '2,8p' "$0"; exit 2 ;;
    esac
done

case $profile in
    basic | voron | happy-hare | ercf) ;;
    *) echo "profile must be basic, voron, happy-hare or ercf" >&2; exit 2 ;;
esac

cd "$root"
cargo build -q -p fake-moonraker -p ui-snapshot

# Stale instances from an earlier run on this port would hold it; instances on other ports are left alone.
pkill -f "fake-moonraker --port $port( |$)" 2>/dev/null || true
pkill -f "ui-snapshot --moonraker http://127.0.0.1:$port( |$)" 2>/dev/null || true
rm -rf "$out_dir"

fake_arguments=(--port "$port" --time-scale 20 --profile "$profile")
if [[ ,$scenes, == *,login,* ]]; then
    fake_arguments+=(--auth test:secret)
fi
target/debug/fake-moonraker "${fake_arguments[@]}" > /dev/null 2>&1 &
fake=$!
trap 'kill "$fake" 2>/dev/null || true' EXIT

for _ in $(seq 50); do
    if (echo > "/dev/tcp/127.0.0.1/$port") 2> /dev/null; then
        break
    fi
    sleep 0.1
done

arguments=(--moonraker "http://127.0.0.1:$port" --out "$out_dir" --size "$size" ${fit_flag[@]+"${fit_flag[@]}"})
if [[ -n $scenes ]]; then
    arguments+=(--scenes "$scenes")
fi
target/debug/ui-snapshot "${arguments[@]}"
