# Open-Source Release (MIT, GitHub) — Design

**Date:** 2026-08-27
**Status:** proposed
**Size:** a repo-hygiene and publication job. No solver behaviour changes.
**Depends on:** nothing in the solver. Applies to `main` as it stands (`7f6ee8b`).

## 1. What is being decided

`psolve` goes public under MIT at `github.com/astroops-cloud/psolve`. The code
is already MIT-licensed and structurally clean; what stands in the way is
**identity and topology**, not secrets, plus the absence of the things a
stranger needs (a runnable first step, a CI that runs for them, contributor
rules).

Measured on `main` at `7f6ee8b`:

| property | value | method |
|---|---:|---|
| tracked files | 128 | `git ls-files \| wc -l` |
| ... of which `.rs` / `.md` / `.toml` | 64 / 43 / 5 | extension tally over `git ls-files` |
| commits, `main` | 233 | `git rev-list --count main` |
| commits, all refs | 249 | `git rev-list --all --count` |
| `.git` size | 12 MB | `du -sh .git` |
| largest blob in history | 2.9 MB (`agreement-postfix.ndjson.gz`) | `rev-list --objects` + `cat-file --batch-check` |
| tests | 631 | `cargo test --workspace -- --list \| grep -c ': test$'` |
| binaries / FITS / star DBs committed | none | extension tally, blob-size sweep |

A secret-shaped scan over all history matched nothing. Stated as what it is:
an enumeration, and **an enumeration that fails open protects only the past**.
It is evidence, not proof, and it is not the criterion this design relies on.

### 1a. Correction to the inventory that arrived from the vault session

A peer session reported `311 .rs, 136 .md, 18 .py, 10 .toml` and
`<home>` in 1,804 places. Both figures are measured over the wrong
population and are corrected above:

- The extension tally counted the **working directory**, which holds two
  gitignored in-repo git worktrees (`.claude/worktrees/`, `.worktrees/`), each
  a full copy of the crate tree, plus `target/`. The repo has 128 tracked
  files and **no Python beyond `scripts/agreement-report.py`**.
- `1,804` counts **file-revisions** across `--all`: one doc mentioning the
  path scores once per commit that ever touched it. The number that governs
  the work is the HEAD surface, which is **20 files** (§3).

Both are recorded rather than quietly replaced, and neither changes the
route chosen. The peer's operational findings (§5) were verified independently
and stand.

## 2. Decisions taken

| decision | choice | note |
|---|---|---|
| destination | `github.com/astroops-cloud/psolve`, **staged private, flipped public after verification** | |
| history | **one squashed initial commit** | taken against this document's author's recommendation; see §2a |
| `docs/superpowers/` | ships **as-is, name unchanged** | zero link churn |
| branches | `main` only | plus one exception, §7 |
| licence | MIT, `Copyright (c) 2026 NRF`, unchanged | |
| scrub depth | HEAD tree, all identity and topology | §3 |

### 2a. The squash, and the cost it carries

The public repo starts at one commit. The GitLab repo keeps all 249 and
remains the archaeology.

The cost is specific: **54 short-SHA citations inside the published docs**
(`297961b`, `3ba1c32`, `5c2e73b`, …) are the provenance behind measured
claims, and in a squashed public repo they resolve to nothing a reader can
reach. This is the exact property those documents exist to provide.

Mitigation, and it is a mitigation rather than a fix: a single stated note
(§8) that public history begins at `v0.1.0` and that SHAs cited in these
documents refer to the pre-release history retained privately. The docs stop
promising a reader something they cannot have.

Not chosen, and why it was on the table: `filter-repo --replace-text` would
have kept a full public history with topology mapped out, and its SHA damage
is mechanically repairable from `.git/filter-repo/commit-map`. It was
declined; recorded here so a later reader knows it was weighed, not missed.

### 2b. What each repo is afterwards

From `v0.1.0` forward **GitHub is canonical**: ordinary, public, per-commit
history, worked from `~/Projects/github.com/AstroOps-Cloud/psolve`. The GitLab
repo is frozen as the pre-release record -- it keeps all 249 commits and stays
the only place the 54 cited SHAs resolve, which is why §10 checks that it
still does. There is no ongoing squash train; the squash happens once.

The initial commit is authored as `NRF <neil@asdf.systems>`, applied
automatically by the `includeIf` in §5 rather than set by hand.

## 3. The scrub surface: 20 files

Union of tracked files in HEAD matching any of `<home>`, `the workstation`,
`the capture host`, `the self-hosted GitLab`, `<LAN>`, `the former maintainer address`, `host B`
(per-token file counts: 12 / 7 / 5 / 3 / 2 / 1 / 1):

```
.gitlab-ci.yml                                     packaging/README.md
crates/psolve-cli/Cargo.toml                       packaging/homebrew/psolve.rb
crates/psolve-cli/src/astap_args.rs                README.md
crates/psolve-cli/tests/astap_cli.rs               docs/astap-compat.md
crates/psolve-cli/tests/fits_update.rs             docs/superpowers/2026-08-14-astap-format-facts.md
crates/psolve-cli/tests/fixtures/reference.ini     docs/superpowers/2026-08-14-m3-first-real-frame.md
crates/psolve-cli/tests/fixtures/reference.wcs     docs/superpowers/2026-08-15-stratified-selection-results.md
crates/psolve-cli/tests/fixtures/reference-failure.ini   docs/superpowers/2026-08-25-index-depth.md
crates/psolve-cli/tests/fixtures/reference-block.wcs     docs/superpowers/plans/2026-08-13-m1-star-index.md
docs/superpowers/plans/2026-08-14-m3-astap-compat.md     docs/superpowers/specs/2026-08-13-psolve-design.md
```

Twenty files is small enough to **read completely**, which is the property
that makes verification tractable. It is the criterion, not the convenience.

### 3a. Four classes, four methods

**(a) The sidecar fixtures — byte-width is load-bearing.**
`reference.wcs`, `reference-block.wcs`, `reference.ini`,
`reference-failure.ini` carry `SITELAT = <site-lat>`, `SITELONG = <site-long>`,
`SITEELEV`, and `CMDLINE=` / `COMMENT cmdline:` strings holding
`<astap-dir>` and `<mnt>/...`.

These are real `astap_cli` output and the ground truth for
`sidecar_ini.rs` / `sidecar_wcs.rs`. Substitution must be **same-width**:
every card is exactly 80 bytes and `reference-block.wcs` must remain a whole
multiple of 2880, or the byte-exact block test fails. Digits are replaced by
digits, path characters by path characters, nothing is shortened.

`OBJCTRA`, `OBJCTDEC`, `DATE-OBS`, `INSTRUME`, `TELESCOP` and the target names
stay. Those are the science; the site is the address.

**(b) The measurement data — the interior, not the file.**
`docs/superpowers/data/task-11-agreement-{full-9495,sample-300}.ndjson.gz`
hold ~9,800 rows whose `path` and `cmd[0]` spell out the private archive
layout (`<astroops>/archive/fits/DWARFIII/<target>/<date>/...`).
Rewrite those two fields to `<archive>/...`, recompress. Every measured field
-- `frame_id`, `db_ra`, `db_dec`, `naxis*`, `binning`, results -- is
untouched, so the ten documents that cite these files still cite real data.
Deleting the files instead would break those ten citations, which is the same
damage §2a is already paying for once.

**(c) Homelab topology in prose.** `the workstation`, `the capture host`, `host B` in benchmark
tables and machine rows become neutral labels that keep the distinction the
tables depend on (two hosts, two architectures) -- `macos-arm64` and
`linux-x86-64` say what the rows are actually contrasting, and say it better.
`<LAN>` and the `the self-hosted GitLab` URLs leave
`packaging/homebrew/psolve.rb` and `packaging/README.md` with the formula
repointed at the GitHub archive URL. Its `sha256` is currently invalid **on
purpose**, so the formula refuses to install rather than installing whatever
it downloaded. That property is kept: the real digest is computed from the
`v0.1.0` tarball only once that tag exists and is reachable, and until then
the deliberate refusal stands with its comment intact.

**(d) Source and test literals.** `astap_args.rs`'s doc comments and unit-test
argv use `<astap-dir>`. One case is not a rename:
`astap_cli.rs:156` deliberately passes **a real directory that holds no
`.psidx`**, and the comment says so. It gets a created temp directory, so the
test keeps testing what it was written to test rather than depending on a path
that exists only on this machine.

## 4. Verification: three checks, and what each can actually prove

`grep` answers "does this string appear", never "is this claim true". So the
criterion is not a clean grep.

1. **Exact-string sweep** over every tracked blob for the seven tokens plus
   the coordinate digit strings. Catches the mechanical misses. Fails open by
   construction -- treated as a filter, not a verdict.
2. **The suite, green at 631.** This is what proves the fixture bytes still
   satisfy the byte-exact format tests after same-width substitution. A scrub
   that broke a card boundary shows up here and nowhere else.
3. **Both re-run against a fresh clone pulled from GitHub while private.**
   The working copy's reflogs and unreachable objects make a local clean
   result meaningless: *the artifact you read is not the artifact in force*.
   The clone is what a stranger receives, so the clone is what gets checked.

Plus a **structural guarantee on what enters the new repo at all**: the public
tree is produced by `git archive main | tar -x`, which by construction emits
only tracked files. The two in-repo worktrees, `target/`, and `.scratch/`
cannot ride along through forgetfulness, because they were never eligible.

## 5. Publishing mechanics (verified, not assumed)

- `gh auth status` — active account is **`astroops-cloud`**; `the personal account` is
  also authenticated and inactive. `gh`'s active account is **global, not
  per-directory**: `gh repo create` uses whoever is active regardless of cwd.
  Check before creating.
- `~/.gitconfig` carries `includeIf "gitdir/i:~/Projects/github.com/AstroOps-Cloud/"`
  → `~/.gitconfig-github-astroops`, so working under that path applies the
  right key and `neil@asdf.systems` automatically. The directory does not
  exist yet.
- The `github-astroops` SSH alias needs its own `ControlPath`: `%C` hashes
  (host, port, user), which is identical for both GitHub aliases, so without
  it the connection rides the other account's master and authenticates as
  `the personal account`. Reported as already fixed in `~/.ssh/config`. It fails
  *toward* the personal account while appearing to succeed, so it is worth
  confirming rather than trusting.
- GitHub push protection matches known provider token formats. It will not
  notice `<home>` or `<LAN>`. It is a backstop, not the plan.

## 6. What ships that does not exist yet

**A runnable first step.** `scripts/demo.sh` plus a self-contained example in
`psolve-cli` that generates a synthetic star field and matching catalogue,
then drives `psolve index build` → `psolve solve` → printed WCS. No download,
no Gaia, no CC BY-NC data in an MIT repo. The generator is lifted from
`cli_solve_success.rs`, which already does exactly this and asserts a real
solve. Becomes the README's first runnable block.

**CI a fork can run.** `.github/workflows/ci.yml`: clippy `-D warnings`, tests
on `ubuntu-latest` and `macos-latest`, packaging on tags only. `.gitlab-ci.yml`
is dropped from the public tree. Two of its lessons migrate into CLAUDE.md and
`packaging/README.md` rather than being lost with it:

- **The suite must not run as root.** `fits_update.rs` stages a failure with
  `chmod 0o311`, which denies root nothing, and it panics naming root rather
  than passing vacuously. GitHub's VM runners are non-root; container jobs are
  not. Use the VM runners.
- **Green proves less than it looks.** GitHub runners have no `~/astroops`
  either, so the same four rig-dependent files skip. `rig_data_dependence.rs`
  keeps that gap from widening silently; the README and CI docs must keep
  saying so.

macOS gains real coverage for the first time: `packaging/README.md` currently
records macOS as neither built nor executed, because Apple SDK licensing rules
out cross-compiling from Linux. A macOS runner changes that statement, so the
statement gets re-measured rather than edited.

**Contributor rules.** `CONTRIBUTING.md`, four things that bite immediately:
do not run `cargo fmt` (819 differing hunks, no CI gate, deliberately);
the structural guards and what they refuse; why some tests skip themselves;
and measured-not-projected as the standard for any claim in a doc.

**Cargo metadata.** `repository`, `homepage`, `readme`, `keywords`,
`categories`, descriptions for `psolve-core` and `psolve-index`, and an MSRV
**measured** against candidate toolchains rather than guessed.

## 7. Branches and tags

Four side branches are superseded and safe to delete once confirmed:
`fix-ci-root` and `pair-match-retry` are 0 ahead; `spike/pair-voting` is one
labelled spike whose successor shipped (`pairmatch.rs` records the two voting
designs that failed first); `binning-retry-refetch` is 14 ahead by patch-id
but its behaviour is on `main` -- `refetch` appears 25× in `cmd_solve.rs` and
`cli_solve_binning_retry_refetch.rs` is tracked there. It landed rebased.

**`fix-cfa-double-binning` is genuinely unmerged and must not be deleted.**
`main:crates/psolve-core/src/fits.rs:160,223` bins any `BAYERPAT` frame 2×2
regardless of `XBINNING`; the branch adds the `xbinning <= 1.0` guard, so a
frame the camera already hardware-binned is not binned a second time in
software. `main`'s own CFA test builds a header with no `XBINNING`, which
defaults to 1.0 under the fix and still yields 2 -- so the fix is **absent,
not rejected**. Whether it is still needed after the 2026-08-23 refetch landed
is an unrun measurement and is out of this job's scope.

The public repo publishes `main` only, tagged `v0.1.0`.

## 8. The provenance note

One short statement, placed where a reader meets the citations
(`docs/astap-compat.md` and the `docs/superpowers/` index point), saying:
public history begins at `v0.1.0`; commit SHAs cited in these documents refer
to the pre-release development history, retained privately; the measurements
themselves are reproducible from the flags and data each document names.

This is the honest form of the trade §2a accepted. It is not a substitute for
resolvable SHAs.

## 9. Sequence, with a stopping point at every boundary

1. Scrub the 20 files in the GitLab working copy; suite green; commit there.
2. `git archive main` → clean tree at
   `~/Projects/github.com/AstroOps-Cloud/psolve`; `git init`; verify the
   tree contains no untracked carry-over.
3. Add metadata, CI, demo, CONTRIBUTING, provenance note; suite green.
4. One commit; tag `v0.1.0`; create the **private** repo with `gh` (confirm
   active account first); push.
5. Fresh clone from GitHub into scratch. Re-run the sweep and the suite there.
   Run the demo there, from nothing but the clone.
6. Flip public.

Steps 1--3 are abandonable at any commit. Step 6 is the only irreversible one,
and it is last on purpose: **publication cannot be re-run if it is wrong**,
which is the whole argument for verifying against the private clone first.

## 10. Acceptance criteria

- [ ] The seven identity/topology tokens return zero matches over every
      tracked blob **in a fresh clone from GitHub**.
- [ ] Site coordinates return zero matches in that clone.
- [ ] `cargo test --workspace` green in that clone and `cargo clippy
      --workspace --all-targets` clean. 631 at the time of writing; if the
      count moves it is **stated in the release commit**, not silently
      different.
- [ ] `scripts/demo.sh` runs to a printed WCS in that clone with no network
      access and no `~/astroops`.
- [ ] The four `.ini`/`.wcs` fixtures still pass their byte-exact tests, and
      `reference-block.wcs` is still a whole multiple of 2880 bytes.
- [ ] The ten documents citing `data/*.ndjson.gz` still resolve to files whose
      measured fields are unchanged.
- [ ] `.github/workflows/ci.yml` green on ubuntu and macOS.
- [ ] `fix-cfa-double-binning` still exists on the GitLab remote.
- [ ] GitLab repo still resolves all 54 cited SHAs.

## 11. Risks

- **A same-width substitution that is not.** A shortened path silently
  breaks an 80-byte card and the format tests catch it -- but only if the
  suite is actually run on the scrubbed tree before the archive step. Ordering
  in §9 exists for this.
- **The sweep is an enumeration.** Seven tokens were chosen from what was
  found; a token nobody thought of is invisible to it. The 20-file read-through
  is the compensating control, and it is only tractable because the surface
  is 20 files.
- **`gh`'s global active account.** Creating the repo from the wrong account
  puts a private repo under `the personal account` and looks like success.
- **The squash is irreversible in one direction only.** Public history cannot
  later be enriched with the private commits without republishing everything
  §2a chose to withhold.

## 12. Deferred by intention

- Publishing a prebuilt index as a release asset (CC BY-NC 3.0 IGO terms, a
  0.23 GB artifact, and a licence boundary to state precisely).
- crates.io publication, including whether the name `psolve` is available.
- Whether `fix-cfa-double-binning` should land (§7) -- a measurement, not a
  tidy-up.
- Rewriting `docs/superpowers/` for a public audience. It ships as-is by
  decision; curating it is a separate job with its own criterion.
