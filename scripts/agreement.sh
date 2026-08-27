#!/usr/bin/env bash
# The Task 11 agreement run: measure psolve against every ASTAP solve this
# machine has actually performed, per
# .superpowers/sdd/2026-08-14-m3-astap-compat/task-11-brief.md.
#
# ~/astroops/ is STRICTLY READ-ONLY. This script never writes into it, never
# passes -update to psolve, and opens catalogue.db with `sqlite3 -readonly`.
#
# Usage:
#   scripts/agreement.sh sample [N] <out.ndjson>   # stratified sample, default N=300
#   scripts/agreement.sh full <out.ndjson>          # every ASTAP-solved frame
#
# Env overrides:
#   PSOLVE_BIN   path to the psolve binary   [default: target/release/psolve]
#   INDEX        path to the .psidx index    [default: ~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx]
#   DB           path to catalogue.db        [default: ~/astroops/state/catalogue.db]
#   JOBS         parallel psolve workers     [default: 8]
#   RIG          restrict to one instrument  [default: all rigs]
#   QUAD_INDEX   .psqidx for the blind fallback  [default: none]
#
# WHY QUAD_INDEX EXISTS. The blind fallback rung only fires when a quad index
# is available, so a run without one cannot exercise it AT ALL -- a regression
# check that never reaches the code it is checking proves only that the change
# is inert. Point this at the .psqidx paired with $INDEX to measure the path
# that actually fires.
#
# WHY RIG EXISTS. The corpus is not evenly distributed across instruments: as of
# 2026-08-27 the DWARFIII contributes 8,105 of the 10,378 ASTAP-solved frames,
# 78% of the total. A corpus-wide solve rate is therefore mostly a statement
# about that one camera, and CANNOT answer "how does psolve do on THIS rig" --
# which is the question that matters when deciding whether to switch a
# particular instrument over. `RIG='ATR585M' scripts/agreement.sh full out.ndjson`
# asks it directly. Valid values come from `SELECT DISTINCT rig FROM frame`.
set -euo pipefail

MODE="${1:?usage: agreement.sh <sample|full> [N] <out.ndjson>}"
if [ "$MODE" = "sample" ]; then
  N="${2:-300}"
  OUT="${3:?usage: agreement.sh sample [N] <out.ndjson>}"
elif [ "$MODE" = "full" ]; then
  OUT="${2:?usage: agreement.sh full <out.ndjson>}"
else
  echo "agreement.sh: mode must be 'sample' or 'full', got '$MODE'" >&2
  exit 2
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PSOLVE_BIN="${PSOLVE_BIN:-$REPO_ROOT/target/release/psolve}"
INDEX="${INDEX:-$HOME/astroops/data/gaia-dr3-g14-dec45-nside64.psidx}"
DB="${DB:-$HOME/astroops/state/catalogue.db}"
JOBS="${JOBS:-8}"
RIG="${RIG:-}"
QUAD_INDEX="${QUAD_INDEX:-}"

if [ ! -x "$PSOLVE_BIN" ]; then
  echo "agreement.sh: $PSOLVE_BIN not built; run cargo build --release first" >&2
  exit 2
fi
if [ ! -f "$INDEX" ]; then
  echo "agreement.sh: index not found at $INDEX" >&2
  exit 2
fi
if [ ! -f "$DB" ]; then
  echo "agreement.sh: catalogue.db not found at $DB" >&2
  exit 2
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# --- Step 1: the frame list. -readonly, never -update, catalogue.db is never
# written. One row per frame_id: a frame filed under both `archive` and
# `library` trees (both intact) is deduplicated by preferring the `library`
# path, deterministically, so the join does not silently multiply a frame
# into two data points. f.ra_deg/f.dec_deg is carried through as the --hint
# psolve is given below -- most of this library's frames (the DWARFIII
# archive tree) have no OBJCTRA/OBJCTDEC header card at all, so a hint
# sourced from the header would silently degrade most of the run to
# FOV_MISMATCH before psolve ever got a chance to solve anything.
#
# f.pointing_src is carried through too, and matters: it is NOT always the
# commanded mount pointing. A fix-round review found 375 frames (371 of them
# `library`-tree) where pointing_src='solve' and frame.ra_deg equals
# measurement.ra_deg to 0.00055" -- ASTAP's own answer copied back as the
# "pointing", not an independent value. agreement-report.py reports this
# count explicitly rather than letting the report imply every hint is
# independent of the answer being measured against.
if [ -n "$RIG" ]; then
  echo "restricting to rig '$RIG'" >&2
  RIG_PREDICATE="AND f.rig = '$(printf %s "$RIG" | sed "s/'/''/g")'"
else
  RIG_PREDICATE=""
fi

echo "extracting ASTAP-solved frame list from $DB ..." >&2
sqlite3 -readonly "$DB" <<SQL > "$WORK/all.tsv"
SELECT l.path, m.ra_deg, m.dec_deg, f.naxis1, f.naxis2, f.binning, f.filt_eff, f.id, f.ra_deg, f.dec_deg, f.pointing_src
FROM measurement m
JOIN frame f ON f.id = m.frame_id
JOIN location l ON l.frame_id = f.id
 AND l.path = (
   SELECT l2.path FROM location l2
   WHERE l2.frame_id = f.id AND l2.intact = 1
   ORDER BY (l2.tree = 'library') DESC, l2.path ASC
   LIMIT 1
 )
WHERE m.tool_version = 'astap/astap+d50'
  AND m.ra_deg IS NOT NULL AND l.intact = 1
  $RIG_PREDICATE;
SQL
TOTAL=$(wc -l < "$WORK/all.tsv" | tr -d ' ')
MEASURED=$(sqlite3 -readonly "$DB" "SELECT COUNT(*) FROM measurement m JOIN frame f ON f.id=m.frame_id WHERE m.tool_version='astap/astap+d50' ${RIG_PREDICATE};")
echo "extracted $TOTAL frames (measurement table has $MEASURED rows for astap/astap+d50)" >&2
if [ "$TOTAL" -gt "$MEASURED" ]; then
  echo "agreement.sh: WARNING extracted more rows ($TOTAL) than measurement has ($MEASURED) -- the location dedup did not fully collapse duplicates" >&2
elif [ "$TOTAL" -lt "$MEASURED" ]; then
  echo "agreement.sh: NOTE $((MEASURED - TOTAL)) measurement rows had no intact location row and were dropped" >&2
fi

# --- Step 2: select frames.
if [ "$MODE" = "sample" ]; then
  echo "stratified sample of $N frames ..." >&2
  python3 - "$WORK/all.tsv" "$N" > "$WORK/selected.tsv" <<'PYEOF'
# Stratified sample across the axes task-11-brief.md section 5 calls out:
# both frame-dimension buckets, OSC vs each mono filter, and the full
# declination range -- AND binning.
#
# Binning was originally NOT stratified, on the stated grounds that "every
# ASTAP-solved frame in this database is binning=1 ... there are no 2x2 rows
# to draw from". That was true when written and expired silently: as of
# 2026-08-22 there are 791 binning=2 rows, and every one of them failed to
# solve. A sampler that omits an axis it assumes is empty cannot see a
# population that appears later. Binning is now a real stratum, and an
# assumed-empty stratum that turns out to be populated is a loud failure
# below, never a silent omission.
#
# Do NOT read this as "the instruments hid the bin-2 hole" -- they did not.
# docs/superpowers/2026-08-15-stratified-selection-results.md recorded it in
# full on 2026-08-15 ("none of them solve, 0/791, a pre-existing gap") and
# deferred it. This guard stops a stratum being omitted silently; it would not
# have caught that one, because that one was never silent.
#
# SCOPE: the guard below checks the BINNING axis only, not the full stratum
# key (dim_bucket, filt_eff, binning). A dimension or filter stratum cut by
# the final [:n] truncation is still omitted silently. Verified 2026-08-23:
# 23 population strata, all present at N=300 and N=500, so this is not biting
# today -- but the next axis to expire silently gets no alarm.
#
# NOTE: this stratum-key change means the fixed seed no longer reproduces
# pre-2026-08-23 300-frame samples cited in
# docs/superpowers/2026-08-14-m3-first-real-frame.md and
# docs/superpowers/2026-08-15-conditional-stratification-results.md.
# Population growth had already broken that independently.
import random
import sys

path, n = sys.argv[1], int(sys.argv[2])
rows = [line.rstrip("\n").split("|") for line in open(path)]
# columns: 0 path 1 db_ra 2 db_dec 3 naxis1 4 naxis2 5 binning 6 filt_eff
#          7 frame_id 8 hint_ra 9 hint_dec 10 pointing_src

def dim_bucket(r):
    return (r[3], r[4])

def stratum(r):
    return (dim_bucket(r), r[6], r[5])

groups = {}
for r in rows:
    groups.setdefault(stratum(r), []).append(r)

random.seed(20260814)  # fixed: the sample must be reproducible, not re-rolled
strata = sorted(groups.keys())
# Proportional allocation with a floor of 1 per non-empty stratum, so every
# dimension x filter combination present in the data appears at least once.
total = len(rows)
alloc = {}
remaining = n
for s in strata:
    k = max(1, round(n * len(groups[s]) / total))
    alloc[s] = k

# Trim/pad to hit n as closely as possible, biggest strata absorb the slack.
def sum_alloc():
    return sum(alloc.values())

strata_by_size_desc = sorted(strata, key=lambda s: -len(groups[s]))
i = 0
while sum_alloc() > n:
    s = strata_by_size_desc[i % len(strata_by_size_desc)]
    if alloc[s] > 1:
        alloc[s] -= 1
    i += 1
    if i > 10000:
        break
i = 0
while sum_alloc() < n:
    s = strata_by_size_desc[i % len(strata_by_size_desc)]
    if alloc[s] < len(groups[s]):
        alloc[s] += 1
    i += 1
    if i > 10000:
        break

selected = []
for s in strata:
    pool = groups[s]
    k = min(alloc[s], len(pool))
    if k >= len(pool):
        chosen = list(pool)
    else:
        # Spread across declination within the stratum rather than a pure
        # random draw, so the full dec range (-90..+24) is represented even
        # within one dimension x filter bucket. Split the dec-sorted pool
        # into k equal-COUNT bins and draw one RANDOM member from each bin --
        # not `round(j*(len-1)/(k-1))` for j in 0..k, which always lands
        # exactly on rank 0 and rank len-1 (the two declination extremes) for
        # every stratum with k>=2, silently over-sampling the extremes on
        # every single draw. A fix-round review of this task traced the
        # 300-frame sample's own skew to exactly this: the sampler, not the
        # solver, over-represented the dec range's edges. Binning still
        # covers the full range (each bin is a contiguous dec slice), just
        # without a deterministic pin to the two endpoints.
        pool_sorted = sorted(pool, key=lambda r: float(r[2]))
        chosen = []
        for j in range(k):
            lo = j * len(pool_sorted) // k
            hi = (j + 1) * len(pool_sorted) // k
            hi = max(hi, lo + 1)
            chosen.append(pool_sorted[random.randrange(lo, hi)])
    selected.extend(chosen)

random.shuffle(selected)
# Truncated FIRST, then checked. The check has to run on the rows that are
# actually written out, not on the pre-truncation list: with a floor of one
# per stratum and enough strata, `selected` can exceed n, and a stratum that
# survives allocation only to be cut by `[:n]` would be just as silently
# omitted as one that was never allocated at all.
emitted = selected[:n] if len(selected) > n else selected

# A stratum that exists in the population must appear in the sample. The
# sampler used to assume binning=2 did not exist; when that assumption
# expired, nothing said so. This makes the same class of mistake loud.
pop_binning = {r[5] for r in rows}
sample_binning = {r[5] for r in emitted}
missing = pop_binning - sample_binning
if missing:
    raise SystemExit(
        f"agreement.sh: sample omits binning stratum/strata {sorted(missing)} "
        f"present in the population -- raise N or fix the sampler; a silently "
        f"omitted stratum is how the 791 bin-2 frames went unseen"
    )

for r in emitted:
    print("|".join(r))
PYEOF
else
  cp "$WORK/all.tsv" "$WORK/selected.tsv"
fi
SELECTED=$(wc -l < "$WORK/selected.tsv" | tr -d ' ')
echo "running psolve over $SELECTED frames, $JOBS parallel workers ..." >&2

# --- Step 3: run psolve over every selected frame and emit one NDJSON line
# each. Defaults everywhere except --index (required) and --hint (see the
# comment above the SQL query for why this run supplies a hint from the DB's
# commanded pointing rather than leaving it to header auto-detection).
PSOLVE_BIN="$PSOLVE_BIN" INDEX="$INDEX" JOBS="$JOBS" QUAD_INDEX="$QUAD_INDEX" \
  python3 - "$WORK/selected.tsv" "$OUT" <<'PYEOF'
import concurrent.futures
import json
import os
import subprocess
import sys
import time

selected_path, out_path = sys.argv[1], sys.argv[2]
PSOLVE = os.environ["PSOLVE_BIN"]
INDEX = os.environ["INDEX"]
QUAD_INDEX = os.environ.get("QUAD_INDEX") or ""
JOBS = int(os.environ.get("JOBS", "8"))

CARD, BLOCK, MAX_BLOCKS = 80, 2880, 256


def parse_header(path):
    """80-byte ASCII cards, 2880-byte blocks, terminated by END. Mirrors
    psolve_core::fits::FitsHeader::parse exactly (see crates/psolve-core/src/fits.rs) --
    astropy is not available on this machine, so this is a from-scratch
    reimplementation, not a shortcut around one."""
    cards = {}
    with open(path, "rb") as f:
        off = 0
        for _ in range(MAX_BLOCKS):
            block = f.read(BLOCK)
            if len(block) < BLOCK:
                break
            done = False
            for i in range(0, BLOCK, CARD):
                card = block[i : i + CARD]
                key = card[:8].decode("ascii", "replace").strip()
                if key == "END":
                    done = True
                    break
                if len(card) > 9 and card[8:9] == b"=":
                    raw = card[9:].decode("ascii", "replace")
                    v = raw.strip()
                    if v.startswith("'"):
                        rest = v[1:]
                        end = rest.find("'")
                        v = rest[:end].strip() if end >= 0 else rest.strip()
                    elif "/" in v:
                        v = v.split("/", 1)[0].strip()
                    if key and key not in cards:
                        cards[key] = v
            off += BLOCK
            if done:
                break
    return cards


def hnum(cards, key):
    v = cards.get(key)
    if v is None:
        return None
    try:
        x = float(v.replace("D", "E").replace("d", "E"))
    except ValueError:
        return None
    return x if x == x and abs(x) != float("inf") else None


def header_ground_truth(path):
    """Returns a dict: header CRVAL/CD when present (only the `library` tree
    carries ASTAP's solved WCS back into the FITS header -- most of this
    library's frames do not), plus the header-optics expected plate scale,
    which IS present on nearly every frame and is exactly the quantity the
    known CFA-2x-FOV hazard shows up in."""
    cards = parse_header(path)
    out = {"has_wcs": False}
    crval1, crval2 = hnum(cards, "CRVAL1"), hnum(cards, "CRVAL2")
    cd = [[hnum(cards, k) for k in row] for row in (["CD1_1", "CD1_2"], ["CD2_1", "CD2_2"])]
    if crval1 is not None and crval2 is not None and all(all(v is not None for v in row) for row in cd):
        det = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0]
        out.update(
            has_wcs=True,
            crval=[crval1, crval2],
            cd=cd,
            header_scale_arcsec=(abs(det) ** 0.5) * 3600.0,
            header_parity="mirrored" if det >= 0.0 else "normal",
        )
    # Expected plate scale from optics keywords alone, matching what
    # psolve_core::fits::pixel_scale_arcsec() computes: 206.265 * pixel_um *
    # XBINNING / FOCALLEN. NOT doubled for a Bayer/CFA frame here, even
    # though decode() does 2x2-superpixel-bin one internally and solve()
    # doubles this same quantity for its OWN internal matching-stage prior
    # (see solve.rs: `scale_arcsec = pixel_scale_arcsec(header) *
    # img.binned`). That doubled value never reaches the CLI's JSON output:
    # the reported WCS is rescaled back to FILE-pixel terms before being
    # returned (solve.rs's own comment: "A consumer applies the WCS to the
    # FILE"), so `field.scale_arcsec` is always in raw file-pixel units.
    # Verified against two real CFA rigs in this corpus (DWARFIII 150mm/2um,
    # SVBONY SV405CC 243mm/4.63um): psolve's reported scale_arcsec matches
    # the UNDOUBLED formula to ~1-2%, and is 2x off the doubled one -- an
    # early version of this script doubled here and manufactured a
    # ubiquitous "scale outlier" that was a ground-truth bug in the harness,
    # not a psolve defect.
    focal = hnum(cards, "FOCALLEN")
    pix = hnum(cards, "XPIXSZ")
    if pix is None:
        pix = hnum(cards, "YPIXSZ")
    xbin = hnum(cards, "XBINNING")
    if xbin is None or xbin < 1:
        xbin = 1.0
    is_cfa = "BAYERPAT" in cards
    if focal is not None and pix is not None and focal > 0 and pix > 0:
        out["expected_scale_arcsec"] = 206.265 * pix * xbin / focal
    out["is_cfa"] = is_cfa
    return out


def run_one(row):
    parts = row.rstrip("\n").split("|")
    (path, db_ra, db_dec, naxis1, naxis2, binning, filt_eff, frame_id, hint_ra, hint_dec,
     pointing_src) = parts
    rec = {
        "path": path,
        "frame_id": int(frame_id),
        "db_ra": float(db_ra),
        "db_dec": float(db_dec),
        "naxis1": int(naxis1),
        "naxis2": int(naxis2),
        "binning": int(binning),
        "filt_eff": filt_eff,
        "hint_ra": float(hint_ra),
        "hint_dec": float(hint_dec),
        "pointing_src": pointing_src,
    }
    try:
        rec["header"] = header_ground_truth(path)
    except OSError as e:
        rec["header"] = {"has_wcs": False, "error": str(e)}

    cmd = [PSOLVE, "solve", path, "--index", INDEX, "--hint", f"{hint_ra},{hint_dec}"]
    # The blind fallback rung only fires when a quad index is available, so a
    # run without one cannot exercise it. Opt in with QUAD_INDEX so the choice
    # is visible in `rec["cmd"]` rather than implied by the environment.
    if QUAD_INDEX:
        cmd += ["--quad-index", QUAD_INDEX]
    rec["cmd"] = cmd
    t0 = time.monotonic()
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        wall_s = time.monotonic() - t0
        rec["exit_code"] = p.returncode
        rec["wall_s"] = wall_s
        stdout = p.stdout.strip()
        try:
            rec["psolve"] = json.loads(stdout) if stdout else None
        except json.JSONDecodeError:
            rec["psolve"] = None
            rec["parse_error"] = "stdout was not valid JSON"
            rec["stdout_raw"] = stdout[:2000]
        if p.returncode not in (0, 1):
            rec["stderr_tail"] = p.stderr[-2000:]
    except subprocess.TimeoutExpired:
        rec["exit_code"] = None
        rec["wall_s"] = time.monotonic() - t0
        rec["psolve"] = None
        rec["parse_error"] = "timeout"
    return rec


rows = [line for line in open(selected_path) if line.strip()]
n = len(rows)
done = 0
t_start = time.monotonic()
with open(out_path, "w") as out_f:
    with concurrent.futures.ThreadPoolExecutor(max_workers=JOBS) as ex:
        for rec in ex.map(run_one, rows):
            out_f.write(json.dumps(rec) + "\n")
            out_f.flush()
            done += 1
            if done % 25 == 0 or done == n:
                elapsed = time.monotonic() - t_start
                print(f"  {done}/{n} ({elapsed:.0f}s elapsed)", file=sys.stderr)

print(f"done: {done}/{n} frames processed", file=sys.stderr)
PYEOF

DONE=$(wc -l < "$OUT" | tr -d ' ')
echo "wrote $DONE NDJSON lines to $OUT (selected $SELECTED)" >&2
if [ "$DONE" != "$SELECTED" ]; then
  echo "agreement.sh: WARNING row count mismatch -- $SELECTED selected, $DONE emitted" >&2
  exit 1
fi
