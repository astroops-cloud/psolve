#!/usr/bin/env python3
"""Task 11 agreement report: read the NDJSON scripts/agreement.sh produced and
print the go/no-go numbers against ASTAP's own 9495 solves.

See .superpowers/sdd/2026-08-14-m3-astap-compat/task-11-brief.md.

Usage: scripts/agreement-report.py <file.ndjson> [file2.ndjson ...]

Exit code: 0 if the gate below passes, 1 otherwise. A failed gate is a
finding, not a script bug -- this still prints the full report before it
exits non-zero.
"""
import json
import math
import statistics
import sys

# ---------------------------------------------------------------------------
# THE GATE. Set BEFORE this script was ever run against real output, per the
# brief's own instruction: "A gate chosen after seeing the numbers is not a
# gate." Do not edit these to make a result pass.
MIN_SOLVE_RATE = 0.95        # psolve solved / ASTAP solved
MAX_MEDIAN_SEP_ASEC = 5.0
MAX_P99_SEP_ASEC = 30.0
MAX_GROSS_ERRORS = 0         # any solve >30" from ASTAP is disqualifying
MAX_PARITY_ERRORS = 0
GROSS_ERROR_ASEC = 30.0
SCALE_OUTLIER_FRAC = 0.05    # 5% deviation flags a frame
# ---------------------------------------------------------------------------


def angsep_arcsec(ra1, dec1, ra2, dec2):
    ra1, dec1, ra2, dec2 = (math.radians(v) for v in (ra1, dec1, ra2, dec2))
    dra = ra2 - ra1
    ddec = dec2 - dec1
    a = math.sin(ddec / 2) ** 2 + math.cos(dec1) * math.cos(dec2) * math.sin(dra / 2) ** 2
    c = 2 * math.asin(min(1.0, math.sqrt(max(0.0, a))))
    return math.degrees(c) * 3600.0


def pctl(values, p):
    """p in [0, 100]. Linear interpolation between closest ranks, matching
    statistics.quantiles(method='inclusive') behaviour at the endpoints."""
    if not values:
        return float("nan")
    s = sorted(values)
    if len(s) == 1:
        return s[0]
    k = (len(s) - 1) * (p / 100.0)
    f, c = math.floor(k), math.ceil(k)
    if f == c:
        return s[int(k)]
    return s[f] * (c - k) + s[c] * (k - f)


def load(paths):
    recs = []
    for path in paths:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line:
                    recs.append(json.loads(line))
    return recs


def is_osc(rec):
    """CFA (Bayer-mosaic) classification by `BAYERPAT` header presence, not
    `filt_eff == 'OSC'`. The two disagree on 206 real frames: SVBONY
    SV405CC frames shot through its Duo-Band filter carry `filt_eff ==
    'Duo-Band'` (a filter name) but ARE a CFA sensor (`BAYERPAT='GRBG'`) --
    16% of the 1305-frame set `filt_eff == 'OSC'` alone would have called
    "mono". `is_cfa` comes from `header_ground_truth()` in agreement.sh,
    which reads `BAYERPAT` directly off the frame's own header, so this
    reclassification does not depend on filter-name bookkeeping at all.
    Falls back to `filt_eff` only if a header couldn't be read at all (should
    not happen for any frame that reached a solve attempt)."""
    h = rec.get("header")
    if h is not None and "is_cfa" in h:
        return bool(h["is_cfa"])
    return rec.get("filt_eff") == "OSC"


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    recs = load(sys.argv[1:])
    n = len(recs)
    if n == 0:
        print("agreement-report.py: no records to report on", file=sys.stderr)
        return 2

    print("=" * 78)
    print(f"Task 11 agreement report -- {n} frames attempted")
    print("=" * 78)

    # --- solve rate -------------------------------------------------------
    solved, not_solved, errored = [], [], []
    for r in recs:
        p = r.get("psolve")
        if p and p.get("solved") is True:
            solved.append(r)
        elif p and p.get("solved") is False:
            not_solved.append(r)
        else:
            errored.append(r)
    solved_ids = {id(r) for r in solved}
    solve_rate = len(solved) / n
    print(f"\n-- Solve rate --")
    print(f"ASTAP solved (input set):        {n}")
    print(f"psolve solved:                   {len(solved)}  ({solve_rate:.4f})")
    print(f"psolve did NOT solve:            {len(not_solved)}")
    print(f"psolve process/parse errors:     {len(errored)}")
    if not_solved:
        reasons = {}
        for r in not_solved:
            reason = (r.get("psolve") or {}).get("reason", "?")
            reasons[reason] = reasons.get(reason, 0) + 1
        for reason, c in sorted(reasons.items(), key=lambda kv: -kv[1]):
            print(f"    {reason}: {c}")
    if errored:
        errs = {}
        for r in errored:
            e = r.get("parse_error", "unknown")
            errs[e] = errs.get(e, 0) + 1
        for e, c in sorted(errs.items(), key=lambda kv: -kv[1]):
            print(f"    {e}: {c}")

    # --- separation vs DB centre (all solved frames; universal ground truth
    #     -- measurement.ra_deg/dec_deg is ASTAP's solved CENTRE for every
    #     one of the 9495 rows, confirmed byte-for-byte equal to header
    #     CRVAL on the subset that carries header WCS) -------------------
    db_seps = []
    for r in solved:
        c = r["psolve"]["field"]["center"]
        sep = angsep_arcsec(c["ra"], c["dec"], r["db_ra"], r["db_dec"])
        r["_db_sep_arcsec"] = sep
        db_seps.append(sep)

    print(f"\n-- Separation vs ASTAP DB centre (arcsec), {len(db_seps)} solved frames --")
    if db_seps:
        print(f"  median: {statistics.median(db_seps):.3f}")
        print(f"  p90:    {pctl(db_seps, 90):.3f}")
        print(f"  p99:    {pctl(db_seps, 99):.3f}")
        print(f"  max:    {max(db_seps):.3f}")

    # --- separation vs header WCS CRVAL, where the header actually carries
    #     ASTAP's solution. Two distinct counts here, not one conflated
    #     figure (a fix-round review caught this file reporting the SOLVED
    #     subset's count as if it were the total): `all_header_wcs_recs` is
    #     every frame (solved or not) whose header carries CRVAL/CD, and
    #     `header_wcs_recs` is the subset of those psolve actually solved
    #     (only that subset has a `field.center` to compare against CRVAL).
    #     Header WCS is overwhelmingly, not exclusively, a `library`-tree
    #     thing: 371 of 375 are `library`-tree, but 4 are `archive`-tree --
    #     the earlier "only on the library tree" claim was wrong. ---------
    all_header_wcs_recs = [r for r in recs if r.get("header", {}).get("has_wcs")]
    header_wcs_recs = [r for r in solved if r.get("header", {}).get("has_wcs")]
    header_wcs_archive = sum(1 for r in all_header_wcs_recs if "/library/" not in r["path"])
    hdr_seps = []
    for r in header_wcs_recs:
        c = r["psolve"]["field"]["center"]
        crval = r["header"]["crval"]
        sep = angsep_arcsec(c["ra"], c["dec"], crval[0], crval[1])
        r["_hdr_sep_arcsec"] = sep
        hdr_seps.append(sep)
    print(f"\n-- Separation vs header WCS CRVAL (arcsec) --")
    print(f"  frames with header WCS present: {len(all_header_wcs_recs)} / {n} "
          f"({len(all_header_wcs_recs)/n:.1%}) -- {len(all_header_wcs_recs) - header_wcs_archive} "
          f"`library`-tree, {header_wcs_archive} `archive`-tree; the rest have no CRVAL/CD "
          f"cards at all")
    print(f"  of those, {len(header_wcs_recs)} were solved by psolve and have a "
          f"field.center to compare against CRVAL (the separation stats below are over "
          f"this solved subset, not all {len(all_header_wcs_recs)})")
    if hdr_seps:
        print(f"  median: {statistics.median(hdr_seps):.3f}")
        print(f"  p90:    {pctl(hdr_seps, 90):.3f}")
        print(f"  p99:    {pctl(hdr_seps, 99):.3f}")
        print(f"  max:    {max(hdr_seps):.3f}")
    else:
        print("  (no frames with header WCS in this run)")

    # --- gross errors (the gate's primary check, against DB centre) ------
    gross = [r for r in solved if r["_db_sep_arcsec"] > GROSS_ERROR_ASEC]
    print(f"\n-- Disagreements over {GROSS_ERROR_ASEC:.0f}\" (gross errors), by DB centre --")
    print(f"count: {len(gross)}")
    for r in sorted(gross, key=lambda r: -r["_db_sep_arcsec"]):
        print(f"    {r['_db_sep_arcsec']:8.2f}\"  {r['path']}")

    # --- scale ratio -------------------------------------------------------
    # psolve's fitted scale_arcsec (from the actual CD determinant, always
    # in FILE-pixel terms -- see solve.rs) against the header-optics-derived
    # expected scale (FOCALLEN/XPIXSZ; see header_ground_truth() in
    # agreement.sh for why this is NOT doubled for a CFA frame -- an earlier
    # version of this script doubled it, replicating solve()'s internal
    # binned-grid match-prior formula rather than what field.scale_arcsec
    # actually reports, and manufactured a false ~2x "outlier" on 7/8 CFA
    # frames in a smoke test before that was found and fixed). This is the
    # CFA-2x-FOV hazard detector the previous milestone's review motivated.
    scale_pairs = []
    for r in solved:
        exp = r.get("header", {}).get("expected_scale_arcsec")
        got = r["psolve"]["field"]["scale_arcsec"]
        if exp and exp > 0 and got:
            ratio = got / exp
            r["_scale_ratio"] = ratio
            scale_pairs.append((ratio, r))
    ratios = [p[0] for p in scale_pairs]
    print(f"\n-- Scale ratio (psolve fitted / header-optics expected) --")
    print(f"frames with an expected scale: {len(scale_pairs)} / {len(solved)} solved")
    if ratios:
        print(f"  median: {statistics.median(ratios):.4f}")
        print(f"  p90:    {pctl(ratios, 90):.4f}")
        print(f"  p99:    {pctl(ratios, 99):.4f}")
        print(f"  min:    {min(ratios):.4f}")
        print(f"  max:    {max(ratios):.4f}")
    scale_outliers = [
        r for (ratio, r) in scale_pairs if abs(ratio - 1.0) > SCALE_OUTLIER_FRAC
    ]
    print(f"scale outliers (>{SCALE_OUTLIER_FRAC:.0%} from header optics): {len(scale_outliers)}")
    for r in sorted(scale_outliers, key=lambda r: -abs(r["_scale_ratio"] - 1.0)):
        print(f"    ratio={r['_scale_ratio']:.4f}  {r['path']}")

    # --- parity ------------------------------------------------------------
    # Only computable where the header carries ASTAP's CD matrix.
    parity_pairs = []
    for r in solved:
        h = r.get("header", {})
        if h.get("has_wcs"):
            got = r["psolve"]["wcs"]["parity"]
            want = h["header_parity"]
            r["_parity_match"] = got == want
            parity_pairs.append(r)
    parity_mismatches = [r for r in parity_pairs if not r["_parity_match"]]
    print(f"\n-- Parity --")
    print(f"frames with header WCS to compare: {len(parity_pairs)}")
    print(f"parity mismatches: {len(parity_mismatches)}")
    for r in parity_mismatches:
        got = r["psolve"]["wcs"]["parity"]
        want = r["header"]["header_parity"]
        print(f"    psolve={got} header={want}  {r['path']}")

    # --- breakdowns ---------------------------------------------------------
    # Every one of the report's headline checks (separation percentiles,
    # scale-ratio distribution, parity), split per bucket -- not just solve
    # rate and a bare median. The scale-ratio-by-OSC/mono split in particular
    # is the one the CFA hazard specifically motivated: a defect that only
    # shows up in the CFA majority would be invisible in an aggregate-only
    # scale-ratio number if the mono minority's numbers happened to look fine.
    def breakdown(label, keyfn, buckets):
        print(f"\n-- Breakdown by {label} --")
        for b in buckets:
            in_b = [r for r in recs if keyfn(r) == b]
            solved_b = [r for r in in_b if id(r) in solved_ids]
            rate = (len(solved_b) / len(in_b)) if in_b else float("nan")
            print(f"  {b}: n={len(in_b)} solved={len(solved_b)} ({rate:.1%})")

            seps_b = [r["_db_sep_arcsec"] for r in solved_b if "_db_sep_arcsec" in r]
            gross_b = [r for r in solved_b if r.get("_db_sep_arcsec", 0) > GROSS_ERROR_ASEC]
            if seps_b:
                print(
                    f"    separation (DB centre, arcsec): median={statistics.median(seps_b):.3f} "
                    f"p90={pctl(seps_b, 90):.3f} p99={pctl(seps_b, 99):.3f} "
                    f"max={max(seps_b):.3f} gross(>30\")={len(gross_b)}"
                )
            else:
                print("    separation (DB centre, arcsec): n/a (nothing solved)")

            ratios_b = [r["_scale_ratio"] for r in solved_b if "_scale_ratio" in r]
            outliers_b = [r for r in solved_b if abs(r.get("_scale_ratio", 1.0) - 1.0) > SCALE_OUTLIER_FRAC and "_scale_ratio" in r]
            if ratios_b:
                print(
                    f"    scale ratio (fitted/expected): median={statistics.median(ratios_b):.4f} "
                    f"p90={pctl(ratios_b, 90):.4f} p99={pctl(ratios_b, 99):.4f} "
                    f"min={min(ratios_b):.4f} max={max(ratios_b):.4f} "
                    f"outliers(>{SCALE_OUTLIER_FRAC:.0%})={len(outliers_b)}"
                )
            else:
                print("    scale ratio: n/a (no expected scale available)")

            parity_b = [r for r in solved_b if "_parity_match" in r]
            mismatches_b = [r for r in parity_b if not r["_parity_match"]]
            print(f"    parity: n_comparable={len(parity_b)} mismatches={len(mismatches_b)}")

    binnings = sorted({r.get("binning") for r in recs})
    if len(binnings) == 1:
        print(f"\n-- Breakdown by binning --")
        print(f"  every frame in this run is binning={binnings[0]}x{binnings[0]} "
              f"(the ASTAP-solved set contains no binned frames to contrast against; "
              f"verified separately against catalogue.db)")
    else:
        breakdown("binning", lambda r: r.get("binning"), binnings)

    osc_count = sum(1 for r in recs if is_osc(r))
    mono_count = n - osc_count
    filt_eff_osc_count = sum(1 for r in recs if r.get("filt_eff") == "OSC")
    print(f"\nOSC/mono classified by BAYERPAT header presence (is_cfa), not filt_eff=='OSC': "
          f"{osc_count} CFA / {mono_count} mono this way, vs {filt_eff_osc_count} / "
          f"{n - filt_eff_osc_count} by filt_eff alone "
          f"({osc_count - filt_eff_osc_count} SVBONY Duo-Band frames reclassified from mono to CFA).")
    breakdown("OSC vs mono (by BAYERPAT)", lambda r: "OSC" if is_osc(r) else "mono", ["OSC", "mono"])

    # A CFA frame with no header WCS still gets the universal DB-centre
    # separation check and the scale-ratio check (both work off the DB
    # centre and the header's optics keywords, present on nearly every
    # frame) -- but NOT the header-CRVAL separation check or the parity
    # check, both of which need a real CD matrix in the header. State this
    # plainly: those two checks say nothing about the 86% CFA majority,
    # exactly where the previous milestone's 2x-FOV hazard lived.
    header_wcs_osc = sum(1 for r in header_wcs_recs if is_osc(r))
    print(f"\nHeader-WCS coverage by OSC/mono: {header_wcs_osc} of {len(header_wcs_recs)} "
          f"solved header-WCS frames are CFA. "
          f"The header-CRVAL separation check and the parity check above therefore say NOTHING "
          f"about the CFA majority of this corpus (only the DB-centre separation and scale-ratio "
          f"checks cover CFA frames); they are reported as mono-only corroboration, not as clean "
          f"coverage of the whole set.")

    # Hint provenance: state plainly that not every --hint was independent
    # mount pointing.
    solve_src = sum(1 for r in recs if r.get("pointing_src") == "solve")
    solve_src_library = sum(
        1 for r in recs if r.get("pointing_src") == "solve" and "/library/" in r.get("path", "")
    )
    print(f"\nHint provenance: {solve_src} of {n} frames have `pointing_src='solve'` in "
          f"catalogue.db -- frame.ra_deg/dec_deg for these IS ASTAP's own solved centre copied "
          f"back, not independent mount pointing ({solve_src_library} of those {solve_src} are "
          f"`library`-tree, essentially all of the same subset the header-WCS/parity checks above "
          f"draw from). For the remaining frames the hint is genuine commanded pointing, "
          f"independent of the answer being measured against.")

    # --- timings (context, not gated) --------------------------------------
    walls = [r.get("wall_s") for r in recs if r.get("wall_s") is not None]
    if walls:
        print(f"\n-- Wall-clock per solve (harness-measured, includes process spawn) --")
        print(f"  median: {statistics.median(walls)*1000:.1f} ms")
        print(f"  p90:    {pctl(walls, 90)*1000:.1f} ms")
        print(f"  max:    {max(walls)*1000:.1f} ms")

    # --- the gate ------------------------------------------------------------
    print("\n" + "=" * 78)
    print("GATE (constants fixed before this run; see top of this script)")
    print("=" * 78)
    median_sep = statistics.median(db_seps) if db_seps else float("inf")
    p99_sep = pctl(db_seps, 99) if db_seps else float("inf")
    checks = [
        ("solve_rate", solve_rate, f">= {MIN_SOLVE_RATE}", solve_rate >= MIN_SOLVE_RATE),
        ("median_sep_asec", median_sep, f"<= {MAX_MEDIAN_SEP_ASEC}", median_sep <= MAX_MEDIAN_SEP_ASEC),
        ("p99_sep_asec", p99_sep, f"<= {MAX_P99_SEP_ASEC}", p99_sep <= MAX_P99_SEP_ASEC),
        ("gross_errors", len(gross), f"<= {MAX_GROSS_ERRORS}", len(gross) <= MAX_GROSS_ERRORS),
        ("parity_errors", len(parity_mismatches), f"<= {MAX_PARITY_ERRORS}", len(parity_mismatches) <= MAX_PARITY_ERRORS),
    ]
    all_pass = True
    for name, value, want, ok in checks:
        all_pass = all_pass and ok
        status = "PASS" if ok else "FAIL"
        vs = f"{value:.4f}" if isinstance(value, float) else str(value)
        print(f"  [{status}] {name} = {vs}  (want {want})")

    print()
    if all_pass:
        print("GATE: PASS")
        return 0
    else:
        print("GATE: FAIL")
        return 1


if __name__ == "__main__":
    sys.exit(main())
