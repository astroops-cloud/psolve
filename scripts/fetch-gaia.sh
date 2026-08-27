#!/usr/bin/env bash
# Fetch Gaia DR3 gaia_source and reduce it to the columns psolve needs.
#
# The full corpus is 3,386 files / ~701 GB gzipped. We never keep that: each
# file is streamed, filtered to (ra, dec, pmra, pmdec, phot_g_mean_mag) at or
# brighter than MAX_MAG, appended to a compact CSV, and discarded. Peak disk is
# one file plus the output.
#
# A fixed observatory never sees the whole sky, so MIN_DEC/MAX_DEC drop stars
# that cannot appear in any frame it takes -- less to download and a smaller
# index. Defaults keep everything.
#
# Usage: scripts/fetch-gaia.sh <outdir> [max_mag] [parallel] [min_dec] [max_dec]
set -euo pipefail

OUT="${1:?usage: fetch-gaia.sh <outdir> [max_mag] [parallel] [min_dec] [max_dec]}"
MAX_MAG="${2:-14}"
PAR="${3:-8}"
MIN_DEC="${4:--90}"
MAX_DEC="${5:-90}"
BASE="https://cdn.gea.esac.esa.int/Gaia/gdr3/gaia_source"
LIST="https://gaia.eu-1.cdn77-storage.com/?prefix=Gaia/gdr3/gaia_source/&delimiter=/"

mkdir -p "$OUT"
cd "$OUT"

# 1. Build the file list (S3-style XML, 1000 keys per page, paginate by marker).
if [ ! -s filelist.txt ]; then
  echo "listing gaia_source ..." >&2
  : > filelist.txt
  marker=""
  while :; do
    url="$LIST"; [ -n "$marker" ] && url="$url&marker=$marker"
    curl -sS --retry 3 --max-time 120 "$url" -o page.xml
    grep -oE '<Key>[^<]*</Key>' page.xml | sed 's/<[^>]*>//g' | grep 'GaiaSource_' >> filelist.txt || true
    grep -q '<IsTruncated>true' page.xml || break
    marker=$(tail -1 filelist.txt | sed 's|/|%2F|g')
  done
  rm -f page.xml
fi
echo "$(wc -l < filelist.txt) files to process" >&2

# 2. Stream, filter, discard. One output shard per input file so this is
#    restartable and parallel-safe.
mkdir -p shards
fetch_one() {
  key="$1"; mag="$2"; dmin="$3"; dmax="$4"
  name=$(basename "$key" .csv.gz)
  [ -s "shards/$name.csv" ] && return 0
  curl -sS --retry 3 --max-time 900 "$BASE/$(basename "$key")" \
    | gunzip -c \
    | awk -v m="$mag" -v dmin="$dmin" -v dmax="$dmax" -F, '
        /^#/ { next }
        !hdr { for (i=1;i<=NF;i++) c[$i]=i; hdr=1;
               print "ra,dec,pmra,pmdec,phot_g_mean_mag" > ("shards/'"$name"'.tmp"); next }
        $c["phot_g_mean_mag"] != "" && $c["phot_g_mean_mag"] != "null" && ($c["phot_g_mean_mag"]+0) <= m &&
        $c["dec"] != "" && $c["dec"] != "null" && ($c["dec"]+0) >= dmin && ($c["dec"]+0) <= dmax {
               print $c["ra"] "," $c["dec"] "," $c["pmra"] "," $c["pmdec"] "," $c["phot_g_mean_mag"] \
                     >> ("shards/'"$name"'.tmp") }
      '
  # A file entirely outside the declination range yields a header-only shard;
  # that is a completed file, not a failure, so still mark it done.
  [ -f "shards/$name.tmp" ] || printf 'ra,dec,pmra,pmdec,phot_g_mean_mag\n' > "shards/$name.tmp"
  mv "shards/$name.tmp" "shards/$name.csv"
}
export -f fetch_one
export BASE

# Record what this mirror actually contains. `psolve index build` reads it and
# refuses to build deeper or wider than the mirror holds, or from a mirror
# whose fetch never finished -- without this, an interrupted multi-hour fetch
# (or asking for more depth/width than was fetched) would silently produce a
# short index that looks exactly like a successful build.
#
# Written via a temp file + `mv` (rename, not truncate-and-rewrite) so a
# concurrent reader of shards/mirror.json never observes a half-written file:
# `read_mirror` in psolve treats a present-but-unparseable manifest as
# "refuse to build", which a partial write could otherwise trigger spuriously.
write_manifest() {
  local complete="$1" rows="$2"
  local tmp
  tmp=$(mktemp "shards/.mirror.json.XXXXXX")
  cat > "$tmp" <<JSON
{
  "source": "Gaia DR3 gaia_source",
  "url": "$BASE",
  "fetched_utc": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "max_mag": $MAX_MAG,
  "min_dec": $MIN_DEC,
  "max_dec": $MAX_DEC,
  "epoch": 2016.0,
  "files": $FILES_TOTAL,
  "rows": $rows,
  "complete": $complete
}
JSON
  mv "$tmp" shards/mirror.json
}
FILES_TOTAL=$(wc -l < filelist.txt)

# Write the manifest BEFORE the fetch runs, marked incomplete. Without this,
# an interruption anywhere during the (multi-hour) xargs run below leaves
# shards/ populated with no mirror.json at all, which `read_mirror` treats as
# "bring-your-own directory, no guard applies" -- exactly the silent-short-
# index hole this manifest exists to close. `rows` is unknown until the fetch
# finishes, so it is recorded as 0 here and corrected below.
write_manifest false 0

xargs -P "$PAR" -I{} bash -c 'fetch_one "$@"' _ {} "$MAX_MAG" "$MIN_DEC" "$MAX_DEC" < filelist.txt

# Fetch ran to completion: rewrite the manifest with the real row count and
# "complete": true. If the process is killed before this point, the
# "complete": false manifest above is what a subsequent `index build` sees.
ROWS=$(cat shards/*.csv 2>/dev/null | grep -vc '^ra,' || echo 0)
write_manifest true "$ROWS"

echo "shards written to $OUT/shards" >&2
cat shards/mirror.json >&2
du -sh shards >&2
