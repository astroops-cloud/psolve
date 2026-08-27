#!/bin/sh
# A complete psolve run with no Gaia mirror, no network and no rig data:
# generate a synthetic star field, build an index from the catalogue it was
# generated from, and solve it. Everything lands in a temp directory that is
# removed on exit.
#
# Debug build by default, because this exists to prove the pipeline works and a
# release build is a slow first step for someone who just cloned the repo. The
# timings in README.md are release builds against real frames -- do not read
# this script's wall time as psolve's speed. Set PSOLVE_DEMO_RELEASE=1 for a
# release build.
set -eu

cd "$(dirname "$0")/.."

if [ "${PSOLVE_DEMO_RELEASE:-0}" = "1" ]; then
    PROFILE="--release"
    echo "==> release build"
else
    PROFILE=""
    echo "==> debug build (set PSOLVE_DEMO_RELEASE=1 for release; timings here are not psolve's speed)"
fi

d=$(mktemp -d)
# shellcheck disable=SC2064
trap "rm -rf '$d'" EXIT

echo "==> generating a synthetic field in $d"
# shellcheck disable=SC2086
cargo run $PROFILE -q -p psolve-cli --example synth_field -- "$d"

echo "==> building an index from the catalogue that field came from"
# shellcheck disable=SC2086
cargo run $PROFILE -q -p psolve-cli -- index build \
    --input "$d/cat" --out "$d/demo.psidx" --max-mag 20 --nside 64

echo "==> solving"
# shellcheck disable=SC2086
cargo run $PROFILE -q -p psolve-cli -- solve "$d/field.fits" \
    --index "$d/demo.psidx" --hint 83.822,-5.391 | tee "$d/out.json"

# The demo is only a demo if it actually solved. Without this it would print a
# failure and exit 0, which is the failure mode this repo exists to avoid.
if ! grep -q '"solved":true' "$d/out.json"; then
    echo "==> DEMO FAILED: no solve in the output above" >&2
    exit 1
fi

echo "==> solved"
