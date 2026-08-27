> **Licensing:** an index built from Gaia data is a derivative of it and carries
> **CC BY-NC 3.0 IGO** — attribution required, non-commercial only. psolve's MIT code
> licence does not extend to it. See [data-licence.md](data-licence.md) before you
> publish or share a built index.

# Building a star index

`psolve` needs a HEALPix-indexed, magnitude-sorted binary catalogue (`.psidx`) on
disk before it can solve anything. This document covers how that file gets
made: fetching Gaia DR3, reducing it to a durable local mirror, and building
one or more `.psidx` files from that mirror with `psolve index build`.

Nothing here is Rust. `scripts/fetch-gaia.sh` is a bash script deliberately
kept out of `psolve` itself — downloading 701 GB is not the solver's job, and
keeping it out means `psolve` carries no HTTP dependency.

## The source

Gaia DR3's `gaia_source` table is published as gzipped ECSV at
`https://cdn.gea.esac.esa.int/Gaia/gdr3/gaia_source/`. Measured facts about it:

- **3,386 files, 701.3 GB gzipped**, named by HEALPix level-8 index range
  (`GaiaSource_<start>-<end>.csv.gz`).
- The directory listing at that URL is JS-rendered and not machine-readable.
  The actual file list comes from the CDN's S3-style XML listing endpoint:
  `https://gaia.eu-1.cdn77-storage.com/?prefix=Gaia/gdr3/gaia_source/&delimiter=/`,
  1,000 keys per page, paginated with `&marker=<last-key>`.
- Each file is **ECSV**: roughly 1,000 leading `#` comment lines (a YAML
  header describing units and provenance), then one CSV header row, then data.
  **152 columns total.** `psolve` only ever needs five: `ra`, `dec`, `pmra`,
  `pmdec`, `phot_g_mean_mag` — plus the optional `source_id`, which is not
  needed by the index but is convenient for cross-checking.
- Columns are located **by name**, never by position — `find_columns` in
  `crates/psolve-index/src/gaia.rs` scans the header row for each required
  name. A reordered or reduced header still works.

## Fetch-time cuts do not save download time

`fetch-gaia.sh` downloads and fully decompresses every file in the listing
regardless of the `max_mag`/`min_dec`/`max_dec` arguments — the magnitude and
declination cuts only decide what gets **written** to the reduced shard
afterward. So a shallow `--max-mag 14` fetch takes exactly as long over the
network as a deep `--max-mag 18` one; the only thing a shallow cut saves is
disk space in the output shards. This is why the guidance below is to always
fetch deep: there is no download-time cost to doing so, only a shard-size one,
and shard size is cheap.

A full-corpus fetch is **bandwidth-bound and takes hours** regardless of the
magnitude limit chosen, because the full 701.3 GB has to move over the wire
either way.

## Reduced-shard sizes

At ~68 bytes/row (5-column CSV) against Gaia DR3's ~1.81 billion sources:

| mirror depth | rows | shards on disk |
|---|---:|---:|
| G<14 | ~76M | ~5 GB |
| G<16 | ~212M | ~14 GB |
| **G<18, full sky** | **~512M** | **~35 GB** |

## Running `fetch-gaia.sh`

```
scripts/fetch-gaia.sh <outdir> [max_mag] [parallel] [min_dec] [max_dec]
```

- `max_mag` (default 14) — faintest `phot_g_mean_mag` to keep in the shards.
- `parallel` (default 8) — concurrent `curl | gunzip | awk` pipelines.
- `min_dec`/`max_dec` (default -90/90, i.e. full sky) — declination cut.

Example, fetching the recommended full-sky G<18 mirror with 12 parallel
fetchers:

```sh
./scripts/fetch-gaia.sh ~/gaia-dr3 18 12
```

The script:

1. Builds `<outdir>/filelist.txt` from the XML listing (skipped if that file
   already exists and is non-empty, so a restart does not re-list).
2. For each file, streams it through `curl | gunzip | awk`, filters to the
   five kept columns and the mag/dec cut, and writes
   `<outdir>/shards/<name>.csv`. Peak disk is one input file plus the
   accumulating output — the compressed input is never kept.
3. Is **restartable**: `fetch_one` skips any file whose `shards/<name>.csv`
   already exists, so re-running the same command after an interruption picks
   up where it left off. It is also safe to run with any `-P`-style
   parallelism since each file's output is independent and only renamed into
   place (`.tmp` → `.csv`) once fully written.
4. A file entirely outside the declination range still produces a
   header-only shard and is marked done — that is a completed file, not a
   failure.
5. Writes `<outdir>/shards/mirror.json` **twice**: once before step 2 starts,
   with `"complete": false` and no row count, and again after step 2 finishes,
   with `"complete": true` and the real row count (see below). Both writes are
   a `mktemp` + `mv` into place, never a truncate-and-rewrite, so a reader
   never observes a half-written manifest.

## Gaia's `null` sentinel

Gaia DR3's bulk export represents an unmeasured value with the **literal
string `null`**, not an empty CSV field. This is not an occasional quirk: a
2,000-row sample of real data from `GaiaSource_299573-302248` found

```
pmra: empty=0  literal-null=285  numeric=1715  other=0
mag : empty=0  literal-null=4
```

— zero empty fields, ever. Both `fetch-gaia.sh` and `gaia.rs` treat `null` as
the same "not measured" case as an empty field:

- The awk filter in `fetch-gaia.sh` excludes `null` explicitly alongside `""`
  in both the magnitude and declination guards, so a null-magnitude row
  (unusable — there is nothing to sort or match on) is dropped at fetch time
  rather than written to the shard.
- `gaia.rs`'s `is_missing()` helper treats `null` (case-insensitively) the
  same as an empty field everywhere a value can legitimately be absent: a
  null magnitude means "skip this source" (`Ok(None)`, same as an empty one),
  and a null proper motion becomes `0.0` (same as an empty one) rather than a
  parse error.

A genuinely malformed value — `N/A`, `abc`, anything that is neither empty
nor `null` and still fails to parse — is still rejected as corrupt input; the
sentinel is documented Gaia behaviour, not a licence to launder arbitrary
junk into a plausible-looking zero.

This was found, and then fixed, using this task's own smoke check (Step 2)
against live data, not a unit test — the original fixtures used genuinely
empty fields, which turned out not to represent the real format at all.
Before the fix, one `null` proper-motion value aborted parsing of the
**entire rest of that file** (a single early `?` in `read_ecsv`'s per-line
loop), which — combined with `null` being near-universal in real data —
meant a full fetch would have silently undercounted by roughly two orders of
magnitude. Measured directly, before vs. after, on the identical two files
(`GaiaSource_000000-003111`, `GaiaSource_003112-005263`, both mag≤14,
full sky):

| | valid rows (mag≤14, real) | rows retained | retention |
|---|---:|---:|---:|
| before fix | 34,470 | 49 | **0.14%** |
| after fix | 34,470 | 34,470 | **100%**, 0 parse warnings |

`index build` against the after-fix shards reports `n_records:34470`
(exactly the row count in `mirror.json`), and `index info --verify` reports
`digest_ok:true`. This is now safe to run at full 701 GB scale.

## Building an index

```sh
cargo build --release
./target/release/psolve index build \
  --input <outdir>/shards --out <path/to/index.psidx> \
  --max-mag <F> --min-dec <D> --max-dec <D> --nside <N> --epoch <Y> \
  --name <short-id>
```

`--input` accepts **any directory of plain `.csv` files** — it does not have
to come from `fetch-gaia.sh`. Anyone who already has catalogue data in a CSV
(Tycho-2, a Vizier export, a hand-built list) can point `--input` straight at
it and skip the fetch script entirely, using `--columns` (below) if the
column names differ from Gaia's.

`psolve` has no gzip decoder — the dependency budget is memmap2 + rayon, not a
compression library — so shards must already be plain `.csv`. `index build`
counts any `.gz`/`.bz2`/`.zst` files in `--input` and refuses to build if it
finds any, rather than silently skipping them and producing a short index.

Verify the result:

```sh
./target/release/psolve index info --verify <path/to/index.psidx>
```

Expected: JSON on stdout including `"digest_ok":true`, exit code 0. A
corrupted index or a missing file exits 3; a usage error (bad flag, malformed
range) exits 2. Without `--verify`, `"digest_ok"` is `null` rather than
`false` — the digest was never checked, and `false` would misleadingly read
as "checked and failed".

`index build`'s own JSON result includes `"files_failed"`, the number of
input files that could not be opened or hit a malformed row partway through
(everything parsed before that row is still kept, but the rest of that file
is lost). Any non-zero `files_failed` exits 3 unless `--allow-partial` was
passed, since a partially-lost file is exactly the kind of silently-short
result this whole build is meant to refuse.

## Build your own index

The mirror is generic (Gaia, full sky, one magnitude depth); the index is
tuned to a site, a camera, and a lens. These are the options to tune per
build:

- **`--max-mag`** — depth. Records are stored magnitude-sorted within each
  HEALPix cell, so a deeper index costs disk but **not** solve time — the
  solver only ever reads the brightest few hundred stars per query cell
  regardless of how deep the index goes. There is little reason to build
  shallower than the mirror allows.

- **`--min-dec`/`--max-dec`** — what the site's latitude can actually reach.
  For a site at latitude `φ` with a hard horizon floor of `f` degrees
  altitude, the highest declination ever reachable to the north is
  `φ + 90 − f` (and correspondingly the lowest to the south is `φ − 90 + f`,
  i.e. the mirror image). Add the field's half-diagonal to that, since a star
  just past the geometric limit can still land inside a frame centred just
  short of it, then round up — declination is cheap to over-include and
  expensive to have missed.

  **Worked example, this rig:** site latitude −38.14° (`core/site.py`
  default in the sibling `astroops` repo, no `astroops.toml` override),
  measured northern horizon 10.0° at az 0 (`horizon.json`, 24 points,
  2026-07-30), probe hard floor 15° (`ladder.py`), frame 2.626° × 1.477° with
  a 1.507° half-diagonal.

  | limit | value | why |
  |---|---|---|
  | `--min-dec` | `-90` | the south celestial pole sits 38.14° up — everything south of dec −51.86° is circumpolar and always available, so there is no reason to cut it |
  | `--max-dec` | `45` | φ + 90 − floor = −38.14 + 90 − 10.0 = **+41.86°** at the measured 10° northern horizon; + 1.51° for the frame's half-diagonal → **+43.37°**, rounded up to 45° |

  That `--max-dec 45` cut removes **14.6%** of the celestial sphere
  (the spherical cap north of dec 45° is `(1 − sin 45°)/2 ≈ 14.6%` of the
  sky) — worth taking since those stars can never appear in a frame from this
  site, but it is not a transformative saving, and the honest number belongs
  here rather than a more flattering one.

- **`--nside`** — HEALPix resolution. Higher `nside` means smaller cells and
  fewer stars scanned per query, at the cost of more cells to look up per
  disc search. `--nside 64` gives ~0.92° cells, which against this rig's
  2.63° × 1.48° field means a query typically touches 9–16 cells. A much
  wider field would want a coarser `--nside 32` so a disc query does not have
  to visit an unreasonable number of cells; a narrower field could go finer.

- **`--epoch`** — the catalogue's reference epoch as a decimal year, used
  with `pmra`/`pmdec` for proper-motion propagation. `2016.0` is Gaia DR3's
  reference epoch; a non-Gaia catalogue should use whatever epoch its own
  positions are quoted at.

- **`--columns`** — column name overrides for a non-Gaia catalogue. Defaults
  are Gaia's own names (`ra`, `dec`, `phot_g_mean_mag`, `pmra`, `pmdec`,
  `source_id`); override any subset with `key=name` pairs. A Vizier export,
  for example:

  ```
  --columns ra=RAJ2000,dec=DEJ2000,mag=Vmag,pmra=pmRA,pmdec=pmDE
  ```

  An unrecognised key is rejected rather than silently ignored, since a typo
  here would build the index from the wrong column.

**A different camera or lens changes only the build command.** A longer focal
length wants a deeper `--max-mag`; a much wider field may prefer a coarser
`--nside`. Both are a rebuild from the same mirror — minutes, no network —
provided the mirror was fetched deep and wide enough to begin with.

## The download-once rule

The 701 GB transfer is the expensive artifact; the `.psidx` file is cheap and
derived from it. So the reduced shards under `<outdir>/shards/` are meant to
be kept as a **durable local mirror**: fetch once, and every later change of
camera, lens, or even observing site is an `index build` re-run against the
existing shards, not a re-fetch.

That only holds if the mirror was fetched **wider and deeper than any index
that will ever be built from it**, because its magnitude and declination cuts
are baked into the shard CSVs at fetch time and cannot be widened without
re-downloading. This is why the recommendation is to fetch the full-sky G<18
mirror (~35 GB) even for a single fixed site that will only ever build a
`--max-dec 45` index from it — as established above, a shallow fetch takes no
less time over the network, so there is no cost to fetching deep, only a
benefit later.

`fetch-gaia.sh` writes `<outdir>/shards/mirror.json`, once before the fetch
runs (`"complete": false`, `"rows": 0`) and again once it finishes
(`"complete": true`, the real row count):

```json
{
  "source": "Gaia DR3 gaia_source",
  "url": "https://cdn.gea.esac.esa.int/Gaia/gdr3/gaia_source",
  "fetched_utc": "...",
  "max_mag": 18,
  "min_dec": -90,
  "max_dec": 90,
  "epoch": 2016.0,
  "files": 3386,
  "rows": ...,
  "complete": true
}
```

`psolve index build` reads it (`read_mirror` in
`crates/psolve-cli/src/cmd_index.rs`) and refuses to build an index deeper or
wider than the mirror actually holds. Without this guard, asking for
`--max-mag 18` against a mirror that was only ever fetched to G<14 would
silently produce a short index that looks exactly like a successful build —
the failure mode this whole design exists to avoid. A `mirror.json` that
exists but fails to parse is treated the same as "refuse to build", not as
"absent" — a truncated or hand-edited file must not be able to disable the
guard. A directory with no `mirror.json` at all (a bring-your-own catalogue)
is not held to this check; the guard only applies when a mirror manifest is
actually present.

Two more conditions refuse the build too (exit 3, not 2 — this is an index
problem, not a usage error), because both describe an **interrupted fetch**
rather than a deliberately shallow one:

- `"complete": false` — the fetch that wrote this manifest never finished.
  `"complete"` missing entirely (a manifest from an older `fetch-gaia.sh`, or
  a hand-built one) is treated as `true`, since the file-count check below is
  the guard that actually catches an incomplete fetch either way.
- Fewer `.csv` files present in `--input` than `"files"` records — the fetch
  was killed mid-`xargs`, after the upfront manifest write but before the
  completion rewrite.

`--allow-partial` opts back into building anyway (exit 0) when either of
those would otherwise refuse, and also when one or more input files failed to
parse (see `files_failed` below). The build still reports exactly what
happened either way; the flag only changes the exit code.

### This rig's index

The mirror fetched by `./scripts/fetch-gaia.sh ~/gaia-dr3 18 12` (Step 7,
operator-run — see below) is generic full-sky Gaia DR3 to G<18. This rig's
own index is built from it with the site-tuned values worked out above:

```sh
cargo build --release
./target/release/psolve index build \
  --input ~/gaia-dr3/shards \
  --out ~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx \
  --max-mag 14 --min-dec -90 --max-dec 45 --nside 64 --epoch 2016.0 \
  --name gaia-dr3-g14-dec45-nside64
./target/release/psolve index info --verify ~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx
```

**The full mirror fetch (Step 7) and this rig's real index build (Step 8) are
operator-run, not part of this task** — the fetch alone is a multi-hour,
701 GB, bandwidth-bound job, and the build depends on its output. Both are
deliberately out of scope here; the pipeline was instead validated with a
two-file, ~90-second smoke fetch and a smoke index built from whatever that
produced (see below). This section will be updated with the real
`n_records`, file size, and wall clock once Step 7 and Step 8 are run — those
will be the first real numbers for spec §12.1 ("index depth and resulting
file size", currently an open question, "to be chosen by building two or
three at different `--max-mag` and testing").

### Smoke-check result (this task, not a full build)

Two rounds against the real endpoints, before and after the `null`-sentinel
fix above:

**Before the fix:** `./scripts/fetch-gaia.sh /tmp/gaia-smoke 14 2`, run for
~90 seconds and then killed, produced `filelist.txt` with all 3,386 entries
and 3 fully-written shards (2 more left as `.tmp`, correctly not picked up by
a subsequent `index build`, demonstrating the restart contract). Building an
index from those 3 shards produced `n_records:51` against ~49,600 combined
data rows across the 3 files — the near-total data loss that led to the fix
above.

**After the fix:** a full, uninterrupted fetch of the same two lowest-numbered
files (`GaiaSource_000000-003111`, `GaiaSource_003112-005263`) at the same
`--max-mag 14`:

```sh
./scripts/fetch-gaia.sh /tmp/gaia-smoke2 14 2
# {"files": 2, "rows": 34470}
cargo build --release
./target/release/psolve index build \
  --input /tmp/gaia-smoke2/shards --out /tmp/gaia-smoke2/smoke.psidx \
  --max-mag 14 --nside 64 --name gaia-dr3-g14-smoke2
# {"n_records":34470, ...}
./target/release/psolve index info --verify /tmp/gaia-smoke2/smoke.psidx
# {"digest_ok":true, ...}
```

produced `n_records:34470` — exactly the row count `mirror.json` recorded,
zero parse warnings, `digest_ok:true`, exit 0. That is the **100%
retention** figure in the before/after table above, on the identical files
used for the "before" measurement.
This confirms the build → verify pipeline end-to-end against real Gaia data;
it is not a stand-in for the real rig index above.
