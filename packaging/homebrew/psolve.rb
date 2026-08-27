# Homebrew formula for psolve.
#
# HOSTING: the `url` below points at this repo's own v0.1.0 source tarball on
# GitHub, so it is reachable by anyone. What it is NOT is verified: the
# `sha256` is still the deliberate placeholder (see TO UPDATE), so this formula
# refuses to install until someone computes the real digest of that exact
# tarball. Publishing it also needs a tap repo; see packaging/README.md.
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
  sha256 "REPLACE_ME_WITH_THE_REAL_DIGEST_SEE_COMMENT_ABOVE"
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
    # `psolve --help` exits 0 and prints the version banner. Note that
    # `psolve --version` is NOT a recognised command -- it prints
    # "unknown command" and exits 2 -- so do not "fix" this test to use it.
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
