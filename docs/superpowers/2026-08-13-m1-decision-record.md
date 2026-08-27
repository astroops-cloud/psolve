# M1 decision record

Rulings and deferred findings from the M1 (star index) implementation run,
2026-08-13. Preserved from the execution ledger, which was scratch.

# SDD ledger — plan: docs/superpowers/plans/2026-08-13-m1-star-index.md

Spec: docs/superpowers/specs/2026-08-13-psolve-design.md (read, reachable)
Branch: m1-star-index (created off main at 98dff60 + bf7c9c5)
Repo was on `main` with only the design+plan commits; branched before any implementation.

## Pre-flight conflict scan

### Pairwise: tasks sharing a file or interface

| A → B | A produces | B consumes | finding |
|---|---|---|---|
| T1 → all | `IndexError`, `lib.rs`, placeholder `healpix.rs` | error type | clean |
| T1 → T2 | placeholder `healpix.rs` | replaced wholesale | clean |
| T2 → T3 | `ang2pix_nest`, `npix`, `is_valid_nside` | same file appended | clean |
| T2 → T5 | `npix`, `is_valid_nside` | `format.rs` calls both | **F1: T5's Interfaces block names `RECORD_BYTES` (T4); the code actually uses `healpix::npix`/`is_valid_nside` (T2). Ordering unaffected.** |
| T2 → T6 | `ang2pix_nest`, `npix`, `is_valid_nside` | builder buckets + cell table | clean |
| T3 → T7 | `cells_in_disc` | `brightest_in_disc` | clean |
| T4 → T6 | `pack`, `StarRecord`, `RECORD_BYTES` | builder packs rows | clean |
| T4 → T7 | `StarRecord`, `RECORD_BYTES` | reader decodes | clean |
| T5 → T6 | `Header`, `records_offset_for`, `FORMAT_VERSION` | builder writes header | clean |
| T5 → T7 | `Header::from_bytes`, `cell_table_offset/bytes` | reader validates | clean |
| T6 → T7 | `Builder` (reader tests build fixtures), `sha256` | `verify_digest` | clean |
| T6 → T9 | `Builder`, `BuildStats` | `index build` | clean |
| T7 → T9/T10 | `Index::open`, `header()`, `cell_len`, `verify_digest` | both subcommands | clean |
| T8 → T9 | `read_ecsv`, `GaiaRow` | build reads shards | **F2: T8 makes `source_id` REQUIRED; T11's `fetch-gaia.sh` emits 5-column shards without it, so `index build` fails on its own downloader's output. T11 patches it 3 tasks later.** |
| T9 → T10 | `flag()` in main.rs, `cmd_index.rs` | `info` appended | clean |
| T9 → T9 | `[dependencies] psolve-index` | Step 2 adds it again under `[dev-dependencies]` | **F3: redundant duplicate dependency entry.** |

### Per-task internal consistency

| task | tests vs code | files created vs later touched | finding |
|---|---|---|---|
| T1 | no tests (scaffold), `cargo build` gate | `lib.rs` appended by T4/5/6/7/8 | clean |
| T2 | 5 tests, fixture already committed | fixture re-read by T3 | clean — fixture parses to exactly 144 data rows after its comment block (verified) |
| T3 | 8 tests incl. round-trip + boundary proof | appends to `healpix.rs` | clean |
| T4 | 8 inline tests, all values checked | — | clean |
| T5 | 8 inline tests | — | clean (see F1) |
| T6 | 4 sha256 + 7 builder tests | `sha256.rs` used by T7 | clean |
| T7 | 8 tests | — | clean |
| T8 | 8 tests + fixture | `gaia.rs` patched by T11 | see F2 |
| T9 | 4 CLI tests | `cmd_index.rs` extended by T10 | see F3 |
| T10 | 5 CLI tests | — | clean |
| T11 | smoke only, no unit tests | patches T8's `gaia.rs` | **F4: Step 2 contains an inert placeholder line (`head -2 /dev/null > /dev/null  # placeholder`).** |

### Rulings

- **Ruling F1: documentation-only defect; T5 proceeds unchanged.** The Interfaces block misnames a dependency but the code and task order are correct. I will state the true dependency in T5's dispatch. Cost if wrong: an implementer briefly looks for an unused import.
- **Ruling F2: move `source_id` optionality from T11 into T8.** Shipping a parser that provably fails on our own downloader's output, then fixing it three tasks later, is worse than doing it once — and it removes a forward dependency the plan otherwise carries. T8's dispatch gets the exact optional-`source_id` code and the extra test; T11's Step 3 becomes verification only. Cost if wrong: trivial — T11's step is a no-op and one test moves task.
- **Ruling F3: drop the `[dev-dependencies]` duplicate in T9.** `psolve-index` is already a normal dependency, so integration tests can use it. Cost if wrong: none; re-adding is one line.
- **Ruling F4: T11 drops the inert placeholder line.** It is leftover scaffolding with no effect. Cost if wrong: none.

## Progress

### Ruling F5 (scope change requested by human partner, pre-Task-1)

Question: "Can we let the user download and process their own Gaia data to their preferences?"

**Ruling: yes — fold four build-time options in now, before any code exists.** Three
things were Gaia-hardcoded that should not have been:
- epoch was literal `2016.0` in the CLI, a latent correctness bug for any other
  catalogue (Tycho-2's proper-motion baseline is decades off Gaia's);
- column names were Gaia's, so a `RAJ2000`/`Vmag` catalogue could not be read at all;
- no sky-region limit, so a fixed southern site indexed stars it can never photograph.

Added: `--min-dec`/`--max-dec`, `--epoch`, `--columns` name overrides (all on
`index build`), and the same declination range on `fetch-gaia.sh`. `--input` already
accepted any CSV directory; that is now documented rather than accidental.

Amended: T1 (error variants `BadColumnSpec`, `BadRange`; `MissingColumn` carries a
String), T8 (`ColumnNames`, `RowFilter`, new `find_columns`/`read_ecsv` signatures,
+5 tests), T9 (four flags, +4 CLI tests), T11 (script takes the dec range; the
source_id patch step is gone, superseded by Ruling F2).

Cost if wrong: the options are additive and defaulted to previous behaviour, so a
wrong call here costs unused flags, not rework. Doing it after implementation would
have cost real rework in three tasks.

Task 1: complete (commits c519a73..f083bce, review clean)

**Ruling F6 (Task 2): resolve the plan's self-contradiction in favour of the Definition of Done.**
The brief dictates `nside >= 1 && nside <= 4096` verbatim in `is_valid_nside`, which trips
`clippy::manual_range_contains`; the plan's own Definition of Done requires
`cargo clippy -- -D warnings` clean. Both cannot hold. Ruled for the DoD — rewrote as
`(1..=4096).contains(&nside)`, semantically identical, because a milestone that can never
pass its own lint gate is worse than a one-line deviation from the brief's text. The
implementer reported the conflict instead of silently deviating, which is the behaviour I
want. Cost if wrong: none — the two forms compile to the same check.
Task 2: complete (commits f083bce..e7dc145, review clean — reviewer independently re-derived
  the algorithm in Python and reproduced the documented boundary source exactly)
Task 2: minor (deferred): is_valid_nside has no direct unit test (gap inherited from the brief)

**Ruling F7 (pre-T9, from human partner: "keep the set around, no need to download it more
than once if I change camera/lenses"):** invert the fetch/build split. The 701 GB transfer
is the expensive artifact; the index is cheap and derived. So the reduced shards become a
DURABLE mirror fetched wide and deep (G<18, full sky, ~35 GB) and the index is narrowed at
BUILD time. Fetch-time cuts are baked in and cannot be widened without re-downloading, so
narrowing there is the one irreversible choice.

Added: `fetch-gaia.sh` writes `shards/mirror.json` (cuts, source, date, rows); `index build`
reads it and exits 2 rather than build deeper or wider than the mirror holds — otherwise a
too-deep request yields a silently SHORT index that looks like a successful build. Two new
CLI tests cover it.

**Also fixed a real plan bug found while checking this:** T9's `csv_files` accepted
`.csv.gz`, but the build path only does `File::open` + text parse and psolve has no gzip
decoder (dependency budget is memmap2 + rayon). A `.gz` would parse as binary, fail the
header lookup, and be SKIPPED WITH A WARNING — a short index reported as success. Now
compressed files are counted and reported, and a directory with nothing readable exits 2.

**Corrected my own overstatement:** the plan claimed a `--max-dec 50` cut "discards roughly
a third of the catalogue". Above dec +50 is 11.7% of the sphere; above +45 is 14.6%. Fixed
in the plan — a flattering number in a design doc is a trap for whoever trusts it later.

Rig profile derived from real sources (lat -38.14 from core/site.py; northern horizon 10.0
deg at az 0 from horizon.json; hard floor 15 deg from ladder.py; frame half-diagonal 1.507
deg): `--min-dec -90 --max-dec 45 --max-mag 14 --nside 64 --epoch 2016.0`.

Cost if wrong: a larger mirror than strictly needed (~35 GB vs ~5 GB) and one guard that
could refuse an over-wide build. Both cheap; the alternative is re-downloading 701 GB.
Task 3: complete (commits e7dc145..17adf7d, review clean — reviewer re-derived the Gorski
  formulas term-for-term and verified the variable rename is behaviour-preserving)
Task 3: minor (deferred): disc_is_a_superset_of_a_brute_force_point_test cannot distinguish
  "correct" from "generously over-padded" (brief-authored oracle is unpadded)
Task 3: minor (deferred): padding_bound_covers_every_point is a 20k-point sweep, not an
  exhaustive per-pixel corner check

Note for M2 (not actioned here): known pixel scale gives every image quad a known angular
size, so catalogue quads outside a few percent of it can be rejected before matching. Not
currently explicit in spec section 6.5; carry into the M2 design.

Task 4: fix round 1/5 — review found 1 Critical, 1 Important, 1 Minor. All are defects in
  the PLAN's reference code, not in the implementer's transcription.
  - Critical: DEC_SCALE = 2^31/90 makes dec=+90 compute exactly 2^31, which overflows i32;
    the cast wraps to i32::MIN and decodes as -90. Any row with dec>=90 clamps to 90.0 and
    is stored in the WRONG HEMISPHERE. All 8 tests missed it (they use 89.9999).
    Fixed to i32::MAX/90 with a saturating clamp. Precision unchanged.
  - Important: clamp_pm treated NaN and +/-inf identically (0, no flag). NaN is "missing"
    (~340M Gaia two-parameter sources) and correctly silent; inf is corrupt and must be
    counted. Guard narrowed to is_nan().
  - Minor (fixed, not deferred): no test pinned little-endian, so a symmetric be/le swap
    would have passed. Added a byte-pattern assertion, since the layout is an on-disk
    contract later tasks cast directly.
  Plan corrected at the same time so a re-run cannot reintroduce it.
Task 4: fix round 1/5 (3 addressed, 0 open — dec hemisphere flip, NaN-vs-inf, endianness
  pin; commits 2769c5a..aedcae5)
Task 4: complete (commits e28030f..aedcae5, review clean)
Task 4: minor (deferred): no test drives f32::INFINITY through pack() at the boundary; the
  code path was verified by inspection only
Task 4: minor (deferred): the saturating clamp on dec_scaled is now unreachable given the
  pre-clamp and exact-fit scale — defensive, harmless
Task 5: complete (commits 1b8387c..aacb238, review clean)
Task 5: minor (deferred): try_into().unwrap_or([0;N]) fallbacks in from_bytes are
  unreachable given the length check, but the shape would silently substitute a default
  if a field width later changed — footgun, not a live bug
Task 5: minor (deferred): no test pins actual serialized byte offsets; all 8 exercise
  round-tripping. Coverage gap for whoever writes the Task 6/7 tests.

**Ruling F8: standardise the clippy gate on `--all-targets`.** The reviewer found that
`cargo clippy -p psolve-index -- -D warnings` does not compile `#[cfg(test)]` code, so
several tasks' reported clippy evidence covered production code only. The reviewer
independently ran `--all-targets` at HEAD and it is clean, so nothing is broken — but the
Definition of Done now says `--all-targets` and dispatches will specify it. Cost if wrong:
none; it is strictly a stronger check.

**Ruling F9 (Task 6): reject the review's Important finding — it would discard ~340M stars.**
The reviewer flagged that `push()` guards ra/dec/mag for finiteness but not pmra/pmdec, and
proposed extending the guard. Rejected: Gaia DR3 holds roughly 340 million two-parameter
sources — real stars with a good position and NO proper motion, published as empty/NaN.
Skipping them would drop about a fifth of the catalogue over an absent optional field.

The asymmetry is deliberate: position and magnitude are required, proper motion is optional,
and Task 4's `clamp_pm` already handles the optional case (NaN -> 0 silently; infinity ->
clamped and counted). The reviewer could not see `pack()` from its diff and said so, so the
finding was reasonable on the evidence available and wrong on the facts.

Constructive half: the intended behaviour was implicit, which is why it read as an oversight.
Added two tests pinning both sides (PM-less stars kept; unusable positions skipped AND
counted) plus a comment at the guard. Cost if wrong: if PM really did need to disqualify a
star, we index some stars with a junk PM whose positions are still correct — far cheaper
than dropping a fifth of the catalogue.
Task 6: fix round 1/5 (1 addressed-by-ruling, 0 open; commits 66b374c..5789506, test-only)
Task 6: complete (commits b29e5ca..5789506, review clean)
Task 6: minor (deferred): streaming_matches_one_shot chunks at 7 bytes, never exercising
  sha256 update()'s bulk >=64-byte branch on the streaming side
Task 6: minor (deferred): no builder test uses a large nside where cell-table alignment
  rounding is non-trivial (all use nside 4/8/64)
Task 7: complete (commits 5789506..201e305, review clean — approved DONE_WITH_CONCERNS)
  Both implementer-initiated hardening changes verified REAL by the reviewer: the plan's
  open() bounds-checked only the record region (not the cell table), and never validated
  cell-table monotonicity — the latter panics in BOTH debug and release via cell()'s
  slice when a > b. Plan amended so a re-run cannot reintroduce either.
Task 7: minor (deferred): the report claimed the brief "explicitly anticipated" the bounds
  fix; the reviewer grepped and it does not appear in the brief. Code correct, provenance
  overstated — worth watching for in later reports.
Task 7: minor (deferred): Truncated{expected: u64::MAX} is an odd semantic fit for an
  arithmetic-overflow error; a dedicated variant would read better
Task 7: minor (deferred): cell + 1 wraps if a caller passes cell == u64::MAX, bypassing the
  guard. Unreachable from untrusted bytes (cells only come from cells_in_disc), unchanged
  from the brief's own code.

**Ruling F10 (Task 8): fix both plan-mandated Important findings — fail loudly, not plausibly.**
1. `opt()` turned a non-empty, unparseable proper motion into 0.0, making corrupt data
   indistinguishable from a genuine PM-less star (~340M Gaia two-parameter sources). Empty
   stays a legitimate zero; garbage is now MalformedRow. Consistent with the principle this
   milestone follows everywhere else (mirror guard, gzip guard, record digest): a broken
   input fails loudly rather than producing a plausible-looking wrong result. A warning
   naming file and line is actionable; a silently wrong catalogue is not.
2. `read_ecsv` returned Ok(0) for a file that is empty or entirely comments, so a truncated
   download that lost its header was indistinguishable from a real catalogue with no rows.
   Now an error.
   `source_id` stays deliberately lenient (advisory field, nothing in the index needs it)
   but the leniency is now commented rather than incidental.
Cost if wrong: a catalogue with genuinely junk PM text now fails its shard loudly instead of
importing zeros. That is the intended trade.
Task 8: fix round 1/5 (2 addressed, 0 open; commits 32d4505..08703df)
Task 8: complete (commits 998b9de..08703df, review clean)
Task 8: minor (deferred): with_overrides accepts a valid key with an empty value ("ra="),
  surfacing later as a confusing MissingColumn("") rather than at the typo
Task 8: minor (deferred): read_ecsv's line_no counts comment lines, so a reported line
  number is the raw file line, not the data-row index

Task 9: fix round 1/5 — review found 2 Critical, both reproduced live, both the recurring
  "exit 0 with a bad result" class:
  - invalid JSON on stdout: --epoch NaN emits "epoch":NaN (not a JSON token) and a --name
    containing a quote breaks the string. The implementer had guarded --max-mag for
    finiteness but not its two siblings.
  - a present-but-corrupt mirror.json returned None, which callers treat identically to
    "absent, skip validation" — silently reopening the silently-short-index hole the guard
    exists to close. Absent and Unreadable must be distinct outcomes.
  Also accepted 3 implementer-found defects in the plan's own code: main.rs called
  cmd_index::info which Step 5 never defines; --max-mag NaN passed RowFilter::validate
  (which only range-checks declination) and would build a zero-record index at exit 0;
  --jobs parse failure was laundered by .ok() into the default core count.
Task 9: fix round 1/5 (2 Critical addressed, 0 open; commits 4427459..3e164b9, +6 tests)
Task 9: complete (commits e46ace7..3e164b9, review clean, 16/16 cli tests)
Task 9: minor (deferred): --name guard uses is_ascii_graphic(), excluding space — stricter
  than "printable ASCII" as worded; harmless
Task 9: minor (deferred): flag() takes the next token as a value, so `--input --out x`
  resolves --input to "--out"; still exits 2 but with a confusing message
Task 9: minor (deferred): an extreme --max-mag/--nside pair can push the derived default
  name past 32 chars, firing the name guard before the nside guard

**Ruling F11 (Task 10): bless the implementer's deviation — a failed `--verify` writes nothing
to stdout.** The brief printed a partial, differently-shaped JSON object on the failure path
while exiting 3. Suppression is better: `build`'s ~15 failure paths already use eprintln only,
so silence is this file's established convention; a truncated object is a shape mismatch on
top of a failure; and under "stdout is results" the exit code already carries the signal.
The implementer flagged it rather than making it silently, which is the behaviour I want.
Added a test pinning empty-stdout-on-failure, since the review correctly noted nothing did.
Cost if wrong: a dashboard scraping stdout regardless of exit code loses a "digest_ok":false
signal it should have been reading the exit code for anyway.

Task 10: complete (commits 6fe5d21..cb4ce5f + pin, review clean, 21/21 CLI tests)
Task 10: minor (deferred): no test exercises json_escape/json_number against a crafted
  hostile on-disk header; the guards are verified by reading only
Task 10: fix round 1/5 (1 pin test added; commits cb4ce5f..58c2aa6)

**Ruling F12 (Task 11) — Gaia's missing-value sentinel is the literal string `null`, and my
Ruling F10 was catastrophic against it.** The Task 11 smoke check against real bulk data
surfaced this; I verified it independently before acting:

    2000 real rows from GaiaSource_299573-302248:
      pmra: empty=0  literal-null=285  numeric=1715  other=0
      mag : empty=0  literal-null=4

Gaia NEVER emits an empty field. The empty-string path the parser was designed around does
not fire on real data, and my `gaia_sample.csv` fixture — which uses empty fields — does not
represent the real format. Combined with F10 ("a non-empty unparseable PM is a fatal
MalformedRow"), the first two-parameter source aborts the whole file: >99% row loss.

Fix: `null` (case-insensitive) joins empty as "missing" — skipped row for a null magnitude,
0.0 for a null proper motion. F10 survives intact for genuine garbage like `N/A`. The same
bug is in fetch-gaia.sh's awk, where `"null"+0` evaluates to 0 and so passes the `<= max_mag`
test, writing unusable rows into the mirror.

Sequenced deliberately BEFORE the operator's 701 GB fetch: shipping it after would bake junk
rows into a 35 GB mirror whose only repair is another 701 GB download. Cost if wrong: a
catalogue that genuinely means "null" as a value rather than a sentinel would lose those
rows — no such catalogue exists.

**This is what "run a known-good case first, not third" bought.** Ten tasks of unit tests,
all green, none of which could have caught it: every fixture was written from my assumption
about the format rather than from the format.
Task 11: complete (commits 58c2aa6..39fbf75, review clean — null-sentinel fix verified at
  read_ecsv level plus a real live-data before/after: 0.14% -> 100% retention)
Task 11: minor (deferred): awk's null guard is exact-match while is_missing is
  case-insensitive; "NULL" would slip past the fetch-time filter (harmless, caught at build)
Task 11: minor (deferred): genuine_garbage_is_still_an_error tests parse_row only, not
  read_ecsv's whole-file abort behaviour on real garbage
Task 11: minor (deferred): shard restart check is existence-only, so a shard written by the
  pre-fix awk is reused; harmless since is_missing handles it at build time

ALL 11 TASKS COMPLETE. Operator's 701 GB fetch launched (bg b0arkb0m1) after the null fix.
