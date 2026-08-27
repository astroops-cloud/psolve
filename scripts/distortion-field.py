#!/usr/bin/env python3
"""Is the residual field the same shape frame to frame?

If it is, the distortion is a property of the optics and can be calibrated once
per rig and reused. If it varies frame to frame, it is not, and a per-frame fit
is the only option.

Method, deliberately independent of psolve's own fit: take psolve's WCS, project
Gaia catalogue stars through it to predicted pixel positions, centroid the real
flux near each, and call the difference the residual vector. That is the same
check the repo already uses to verify a solve is not confidently wrong.
"""
import glob, json, os, subprocess, sys
import numpy as np

BIN = "./target/release/psolve"
IDX = os.path.expanduser("~/astroops/data/gaia-dr3-g14-dec45-nside64.psidx")


def read_fits(path):
    """Minimal FITS reader: 2880-byte header blocks, then the data unit."""
    with open(path, "rb") as fh:
        hdr, cards = {}, b""
        while True:
            block = fh.read(2880)
            if not block:
                return None, None
            cards += block
            if b"END     " in block:
                break
        for i in range(0, len(cards), 80):
            c = cards[i:i + 80].decode("latin-1")
            if c.startswith("END "):
                break
            if "=" in c[:10]:
                k = c[:8].strip()
                v = c[10:].split("/")[0].strip().strip("'").strip()
                hdr[k] = v
        try:
            nx, ny = int(hdr["NAXIS1"]), int(hdr["NAXIS2"])
            bitpix = int(hdr["BITPIX"])
        except KeyError:
            return None, None
        if bitpix != 16:
            return None, None            # only the 16-bit rigs here
        raw = fh.read(nx * ny * 2)
        if len(raw) < nx * ny * 2:
            return None, None
        img = np.frombuffer(raw, dtype=">i2").astype(np.float64).reshape(ny, nx)
        img += float(hdr.get("BZERO", 0) or 0)
        return hdr, img


def solve(path):
    try:
        out = subprocess.run([BIN, "solve", path, "--index", IDX],
                             capture_output=True, text=True, timeout=180).stdout
        d = json.loads(out)
    except Exception:
        return None
    return d if d.get("solved") else None


def catalogue(ra, dec, radius, maxmag):
    out = subprocess.run([BIN, "index", "query", IDX, "--ra", f"{ra}", "--dec", f"{dec}",
                          "--radius", f"{radius}", "--max-mag", f"{maxmag}"],
                         capture_output=True, text=True, timeout=120).stdout
    rows = []
    for line in out.splitlines()[1:]:
        p = line.split(",")
        if len(p) >= 3:
            try:
                rows.append((float(p[0]), float(p[1]), float(p[2])))
            except ValueError:
                pass
    return rows


def radec_to_pixel(ra, dec, wcs):
    """Gnomonic (TAN) de-projection, then the inverse CD matrix."""
    ra0, dec0 = np.radians(wcs["crval"][0]), np.radians(wcs["crval"][1])
    ra, dec = np.radians(ra), np.radians(dec)
    cosc = np.sin(dec0) * np.sin(dec) + np.cos(dec0) * np.cos(dec) * np.cos(ra - ra0)
    xi = np.cos(dec) * np.sin(ra - ra0) / cosc
    eta = (np.cos(dec0) * np.sin(dec) - np.sin(dec0) * np.cos(dec) * np.cos(ra - ra0)) / cosc
    xi, eta = np.degrees(xi), np.degrees(eta)
    cd = np.array(wcs["cd"], dtype=float)
    inv = np.linalg.inv(cd)
    dx = inv[0, 0] * xi + inv[0, 1] * eta
    dy = inv[1, 0] * xi + inv[1, 1] * eta
    return dx + wcs["crpix"][0], dy + wcs["crpix"][1]


def centroid(img, x, y, box=6):
    ny, nx = img.shape
    xi, yi = int(round(x)), int(round(y))
    if xi - box < 0 or yi - box < 0 or xi + box >= nx or yi + box >= ny:
        return None
    cut = img[yi - box:yi + box + 1, xi - box:xi + box + 1]
    bg = np.median(cut)
    w = cut - bg
    peak = w.max()
    if peak <= 0:
        return None
    w = np.where(w > 0.25 * peak, w, 0.0)      # core only
    tot = w.sum()
    if tot <= 0:
        return None
    gy, gx = np.mgrid[yi - box:yi + box + 1, xi - box:xi + box + 1]
    cx = (w * gx).sum() / tot
    cy = (w * gy).sum() / tot
    snr = peak / (np.std(cut) + 1e-9)
    return cx, cy, snr


def residual_field(path, grid=4):
    d = solve(path)
    if d is None:
        return None
    hdr, img = read_fits(path)
    if img is None:
        return None
    ny, nx = img.shape
    w = d["wcs"]
    fov = max(d["field"]["fov_deg"])
    cat = catalogue(d["field"]["center"]["ra"], d["field"]["center"]["dec"], fov * 0.75, 13.0)
    if len(cat) < 20:
        return None
    vecs = []
    for ra, dec, mag in cat:
        px, py = radec_to_pixel(ra, dec, w)
        if not (20 < px < nx - 20 and 20 < py < ny - 20):
            continue
        c = centroid(img, px, py)
        if c is None:
            continue
        cx, cy, snr = c
        dxp, dyp = cx - px, cy - py
        if snr < 4 or abs(dxp) > 4 or abs(dyp) > 4:   # not the same star
            continue
        vecs.append((px, py, dxp, dyp))
    if len(vecs) < 15:
        return None
    v = np.array(vecs)
    # bin into grid x grid cells over the frame
    field = np.full((grid, grid, 2), np.nan)
    gx = np.clip((v[:, 0] / nx * grid).astype(int), 0, grid - 1)
    gy = np.clip((v[:, 1] / ny * grid).astype(int), 0, grid - 1)
    for i in range(grid):
        for j in range(grid):
            m = (gx == i) & (gy == j)
            if m.sum() >= 2:
                field[j, i, 0] = v[m, 2].mean()
                field[j, i, 1] = v[m, 3].mean()
    return field, len(vecs), float(np.hypot(v[:, 2], v[:, 3]).mean())


def main(pattern, label, want=12):
    files = sorted(glob.glob(os.path.expanduser(pattern)))
    if not files:
        print(f"{label}: no frames"); return
    step = max(1, len(files) // want)
    fields, ns, means = [], [], []
    for f in files[::step][:want]:
        r = residual_field(f)
        if r is None:
            continue
        fields.append(r[0]); ns.append(r[1]); means.append(r[2])
    if len(fields) < 4:
        print(f"{label}: only {len(fields)} usable frames"); return

    F = np.array(fields)                       # (frames, gy, gx, 2)
    flat = F.reshape(len(F), -1)
    ok = ~np.isnan(flat).any(axis=0)
    flat = flat[:, ok]
    mean_field = flat.mean(axis=0)

    # Frame-to-frame correlation of the residual pattern
    cors = []
    for i in range(len(flat)):
        for j in range(i + 1, len(flat)):
            a, b = flat[i], flat[j]
            if a.std() > 0 and b.std() > 0:
                cors.append(float(np.corrcoef(a, b)[0, 1]))

    scatter = flat.std(axis=0).mean()
    signal = np.abs(mean_field).mean()

    print(f"\n=== {label} ===")
    print(f"  frames used            : {len(F)}   stars/frame median: {int(np.median(ns))}")
    print(f"  mean |residual|        : {np.mean(means):.3f} px")
    print(f"  mean field |component| : {signal:.4f} px   <- the repeatable part")
    print(f"  frame-to-frame scatter : {scatter:.4f} px   <- the noise")
    print(f"  signal / scatter       : {signal/scatter:.2f}")
    if cors:
        c = np.array(cors)
        print(f"  pattern correlation    : median {np.median(c):+.3f}  "
              f"[{np.percentile(c,10):+.2f} .. {np.percentile(c,90):+.2f}]  n={len(c)} pairs")
    print(f"  VERDICT: ", end="")
    if cors and np.median(cors) > 0.5 and signal > scatter:
        print("STABLE -- the residual field repeats. Calibrate once per rig.")
    elif cors and np.median(cors) > 0.25:
        print("PARTLY stable -- a repeatable component exists but noise is comparable.")
    else:
        print("NOT stable -- residuals do not repeat. Per-frame fitting only.")


if __name__ == "__main__":
    main("~/astroops/archive/fits/DWARFIII/**/*.fits", "DWARFIII (strongest radial signal)")
    main("~/astroops/library/*/lights/*/*.fits", "ATR585M (primary rig)")
