# Homebrew formula for psolve.
#
# HOSTING: the `url` points at this repo's own v0.1.0 source tarball on GitHub,
# and the `sha256` below is the REAL digest of it, computed 2026-08-27 and
# checked stable across two independent fetches. It is no longer the refusing
# placeholder this file carried until then.
#
# TWO THINGS THAT STILL BLOCK `brew install`:
#
#   1. A formula does nothing sitting in this directory. It has to live in a
#      repo named `homebrew-<tap>`; see packaging/README.md.
#   2. The digest is of GitHub's AUTO-GENERATED source tarball. Those are not
#      contractually byte-stable -- GitHub has changed its archive compression
#      before and broken formulas doing exactly this. If `brew` ever reports a
#      checksum mismatch, re-compute rather than assuming a compromised
#      download, and consider attaching a hand-built tarball to the release so
#      the bytes are ours rather than generated.
#
# TO PUBLISH: this file has to live in a tap repo named `homebrew-<tap>` --
# it does nothing sitting here. See packaging/README.md for the steps, which
# include creating that repo (an outward action, deliberately not automated).
#
# TO UPDATE: bump `url` to the new tag and replace `sha256` with the real
# digest of that tarball:
#
#     curl -sL <url> | shasum -a 256
#
# The placeholder below is NOT a valid digest and `brew install` will refuse
# it. That is deliberate -- a formula that installs whatever it downloaded is
# worse than one that refuses to install at all.
class Psolve < Formula
  desc "Plate solver: FITS bytes in, a verified TAN WCS out. ASTAP-CLI compatible"
  homepage "https://github.com/astroops-cloud/psolve"
  url "https://github.com/astroops-cloud/psolve/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "db116599cb8d0e2a675b8b4a1ff1f9249fdefc0ee0afbad591720d68a6438f3e"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/psolve-cli")

    doc.install "README.md"
    doc.install "docs/astap-compat.md"
    doc.install "docs/index-building.md"
    doc.install "docs/data-licence.md"
  end

  def caveats
    <<~EOS
      psolve needs a star index and this formula does NOT ship one.

      An index is built from a Gaia DR3 mirror with `psolve index build`, and is
      licensed CC BY-NC 3.0 IGO -- non-commercial, attribution required. That is
      a different licence from psolve's own MIT, which is why no index is
      distributed here. The smallest useful one is about 0.23 GB.

        #{doc}/index-building.md   how to build one
        #{doc}/data-licence.md     what the licence means

      Without an index psolve will refuse to solve anything.
    EOS
  end

  test do
    # `psolve --help` exits 0 and prints the version banner.
    #
    # This comment used to warn that `psolve --version` was NOT a recognised
    # command and exited 2. That stopped being true on 2026-08-24 and the
    # warning was left behind. Measured 2026-08-27: `psolve --version` prints
    # `psolve <version> (<build>)` and exits 0. Either check is valid; --help
    # is kept because it also exercises the banner.
    assert_match "psolve #{version}", shell_output("#{bin}/psolve --help")

    # A solve against paths that do not exist must refuse cleanly rather than
    # crash. Verified 2026-08-23: it fails on the missing FRAME first and exits
    # 2 (usage/config) -- it never reaches the index, so this does not exercise
    # the index-problem code 3. Exit codes are 0 solved, 1 not solved,
    # 2 usage/config, 3 index problem.
    output = shell_output("#{bin}/psolve solve /nonexistent.fits --index /nonexistent.psidx 2>&1", 2)
    refute_empty output
  end
end
