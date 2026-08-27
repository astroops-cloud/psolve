# Contributing

Four rules that will otherwise bite you in the first ten minutes. Each one has
a reason, because an unexplained rule gets "fixed".

## Do not run `cargo fmt`

This repo has never been rustfmt-clean and there is deliberately **no**
`cargo fmt --check` gate in CI. A bare `cargo fmt` rewrites about 60 files --
819 differing hunks, measured 2026-08-26 -- which buries a real change inside a
whole-repo reformat and makes the diff unreviewable.

Format only what you touch, by hand, matching the surrounding code.

Reformatting the tree is a decision that can be made on its own merits, in its
own commit, by someone who wants to argue for it. It is not a side effect of an
unrelated patch.

## Some tests skip themselves, and one file polices that

Tests needing multi-GB indexes or real telescope frames print `skipping` and
pass when that data is absent, so the suite runs on a machine that has none of
it. That convention is right, and it has a cost: on CI, where none of that data
exists, those tests pass without testing anything.

`crates/psolve-cli/tests/rig_data_dependence.rs` pins exactly which files may
do this. Add a fifth and the suite fails until you list it there -- which forces
the choice: commit a fixture, or widen the coverage gap on purpose. Widening it
is allowed. Widening it silently is not.

The consequence for you: **a green CI run means "compiles, clippy-clean,
data-independent tests pass"**. It does not mean the solver still agrees with
ASTAP on a real corpus, or that sidecar bytes still match. Those are measured
against real data by the maintainer.

## The guards fail closed

- `psolve-core` has **no dependencies, not even dev-dependencies**, and may not
  name `fs`/`net`/`process`/`env`/`File`/`OpenOptions`/`PathBuf` anywhere in
  its source -- **including in a comment**. `no_filesystem.rs` scans tokens and
  cannot tell prose from code, so if it fails on a comment, reword the comment.
  This is what makes "the solver cannot modify your data" a property of the
  dependency graph rather than a promise.
- `tree_is_scrubbed.rs` keeps personal paths, private addresses and observation
  coordinates out of a published tree.
- `fixtures_are_tracked.rs` proves the reference sidecars are not caught by
  `.gitignore`'s `*.ini`/`*.wcs` rules.

All three tell you the fix in the failure message. None is negotiable in a PR.

## Measured, not projected

Any number in a document carries the invocation, the flags and the machine
state that produced it. When a re-measurement contradicts an earlier claim,
**both figures are reported and the earlier one retracted in place** -- see the
README's timing section for what that looks like in practice.

Do not smooth over a failed criterion or a disagreement. Investigate it and
record what you found. A wrong explanation offered as a caveat is worse than an
admitted unknown, because it retires the question.

The same standard applies to a claim about the code: `grep` answers "does this
string appear", never "is this claim true".

## Practical notes

- `cargo test --workspace` -- the full suite. `cargo clippy --workspace
  --all-targets` must stay clean.
- `./scripts/demo.sh` -- a complete solve on synthetic data, no index needed.
  If you change the pipeline, this is the fastest end-to-end check.
- **Do not run the suite as root.** `fits_update.rs` stages a permission failure
  with `chmod 0o311`, which denies root nothing, so under root it cannot arrange
  the condition it exists to exercise -- and it panics naming root rather than
  passing vacuously. Do not "fix" that by making the test tolerate root.
- **No MSRV is declared.** It has not been measured, and cargo enforces
  `rust-version` on users, so a guessed floor would be a real defect rather
  than documentation. CI builds on stable. If you need a floor, measure it and
  open an issue with the method.
- Commit subjects are `type(scope): summary`. The message states the finding,
  not the edit.
