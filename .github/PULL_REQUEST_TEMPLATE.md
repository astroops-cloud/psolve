## What this changes

<!-- The finding, not the edit. What is true after this that was not before? -->

## Checks

- [ ] `cargo test --workspace` is green. Test count: <!-- state it; if it moved, say why -->
- [ ] `cargo clippy --workspace --all-targets` is clean
- [ ] I did **not** run `cargo fmt` over the repo (see CONTRIBUTING.md -- it
      rewrites ~60 files and buries the diff). Only what I touched is formatted,
      by hand.

## If this changes solver behaviour

- [ ] `./scripts/demo.sh` still solves
- [ ] A frame that solved before still solves, by the same route -- the retry
      ladder's ordering is load-bearing, and a rung that pre-empts the ones
      above it has shipped here before without anything failing
- [ ] Any number quoted in a doc carries the invocation and the machine state
      that produced it

## If this changes the ASTAP-compatible surface

- [ ] Wired through **both** entry points (`cmd_solve.rs` and ASTAP dispatch),
      not just the one you were looking at
