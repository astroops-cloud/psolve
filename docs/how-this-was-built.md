# How this was built

**Short version:** effectively none of this code was written by a human. It was
built over 14 days with [Claude Code](https://claude.com/claude-code), directed
and reviewed by one person who hand-wrote essentially nothing. 251 commits,
2026-08-13 to 2026-08-27.

I did not set out to write a plate solver. I set out to find out what
vibecoding could actually do, and a plate solver happened to be a good test:
the problem is well defined, the output is checkable against a tool that
already works, and I had 12,620 real frames to check it on. I published it
because the results surprised me.

## What this is not

It is not a criticism of [ASTAP](https://www.hnsky.org/astap.htm). ASTAP is the
industry standard for this job and it earned that. It is a complete astronomy
suite where this is one narrow tool; it ships all-sky databases where this makes
you build your own; it handles distortion where this fits a plain TAN; it has a
decade of other people having hit the bugs first, where this has one
observatory and a fortnight. It has run my pipeline since that pipeline
existed, and it stays installed.

psolve is now going in beside it -- I am eating my own dogfood, as of
2026-08-27. Beside it, not instead of it: my own last per-rig measurement has
psolve losing on my primary camera, and the improvements that landed afterwards
have not been measured against that split. Running it in production is how I
find out whether it holds up; it is not a claim that it already has.

The comparisons in the README are on frames ASTAP **failed** on or has no
record for. That is a biased sample by construction, and it is labelled as one.
The number that is actually fair — psolve reproducing ASTAP's own answers on the
10,376 frames ASTAP solved — is 99.93% agreement at 0.54″ median. That is
agreement, not victory.

## What the AI was good at

- **Volume of careful, boring correctness.** A FITS header parser that never
  panics on hostile input, 80-byte card arithmetic, a `-update` path that writes
  a temp copy, fsyncs, reparses it from a fresh read, requires byte-identical
  pixels and only then renames. That is exactly the kind of code humans get
  bored writing and therefore get wrong.
- **Writing down why.** Nearly every module here carries a doc comment
  explaining the design and, more usefully, the designs that failed first.
  `pairmatch.rs` records two voting schemes that did not work before the one
  that did. That is the most valuable thing in the repo and it cost nothing to
  produce.
- **Building its own guards.** Several tests here exist to make a convention
  fail rather than be remembered: `psolve-core` cannot name a filesystem symbol
  even in a comment; a pinned list of which tests are allowed to skip
  themselves; a check that the reference fixtures are not caught by
  `.gitignore`.
- **Speed.** Fourteen days, evenings and weekends, for something I would have
  estimated in months.

## What it cost

This is the part worth reading if you are considering the same approach. None
of the following was caught by tests passing. All of it was caught by measuring
against real data, and most of it looked fine first.

**A confident answer 87.77° from the truth.** The verification gate
(`verify.rs`) computes log-odds that a match is not chance. It was calibrated
for *one* hypothesis against a known sky region. Blind solving tests thousands
of hypotheses, and reusing the single-hypothesis threshold produced a solve that
was wrong by most of the sky while reporting high confidence. The gate is now
multiplicity-corrected. **Nothing crashed. The output was well-formed.** This is
the defect shape that recurs: not an exception, a plausible return value.

**791 frames that were invisible for eight days.** Every 2×2-binned colour frame
in the corpus failed to solve — 77% of all failures — and the agreement runs did
not show it, because the 9,495-frame corpus being used contained *no such frames
at all* and the sampler that picked test subsets happened to exclude them too.
The instrument and the sample were both blind in the same place. It was recorded
in full in a measurement document eight days before anyone acted on it, deferred
as out of scope, and stayed deferred because nothing looked wrong.

**Claims that were wrong and had to be retracted in place.** "Two thirds" that
was asserted rather than computed. A 98.3% import rate that was really a
42-minute window quoted as a whole-run figure. Cross-host build reproducibility
claimed, then retracted when the digests turned out to differ. A timing ratio
measured under machine conditions that no longer existed. The repo's convention
is that both figures get reported and the earlier one is struck through rather
than quietly edited, which is why you can still read all of them.

**A fix that passes every test and is still wrong.** There is a branch here
fixing a real defect — colour frames that the camera already binned get binned
again in software, halving resolution for nothing. It passes every one of the 633 tests that run. It
also regressed 184 real frames when measured, because an unrelated extraction
threshold had silently tuned itself around the bug. It is documented as a known
limit and deliberately not merged.

**A retry that pre-empted the ones above it.** A new code path defaulted on
inside the core silently moved 41 already-solving frames onto a different route.
They still solved. They still agreed to under 2 arcseconds. Nothing failed —
which is exactly why it nearly shipped.

## What made it survivable

Not the model, and not the tests. Three habits:

1. **Measure against reality, not against the test suite.** Every number in this
   repo carries the command that produced it, the corpus, and the machine state.
   A green suite proves the code compiles and behaves as written; it cannot tell
   you the code is solving the wrong problem.
2. **Treat a plausible return value as the enemy.** Almost every expensive
   defect here returned something that looked fine. Designing for a loud refusal
   rather than a best guess is what turned several of them into visible failures.
3. **Retract in place.** When a re-measurement contradicts an earlier claim,
   both numbers are published and the old one is marked wrong. A wrong
   explanation offered as a caveat is worse than an admitted unknown, because it
   retires the question.

An AI will produce a confident, well-structured, plausible answer whether or not
it is correct. So will a human. The difference is throughput: you get many more
of them, much faster, and the ones that are wrong look exactly like the ones
that are right. Everything above is a mechanism for making that difference
survivable.

## Should you use this?

If you want a GUI, **distortion correction**, or a decade of field-tested
reliability across thousands of setups — use ASTAP. It is genuinely excellent at
all of those, and the distortion gap is real: psolve fits a plain TAN with no
distortion terms and accepts `-sip` only to ignore it.

If you are automating a pipeline and want structured JSON, a reason code
explaining *why* a frame did not solve, one static binary with no runtime, or a
star index tuned to your own sky, then this may suit you, and it is MIT
licensed. Read [CHANGELOG.md](../CHANGELOG.md)'s known-limits section first —
it is honest about what does not work.

Two items moved off that first list on 2026-08-27 and it is worth saying which,
because the list was written when they were true: **Windows** is now built
natively and tested by CI on every push (620 of the 633 run; no human has run it),
and **a prebuilt all-sky index is downloadable** from the release rather than
something you must build. Distortion is the one that has not moved.

Either way, keep a backup of your frames. That advice has nothing to do with who
or what wrote the code.
