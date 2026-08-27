# ASTAP ground-truth facts (gathered 2026-08-14, read-only, this machine)

All values below are quoted verbatim from real files / real command output on this
machine. Nothing here is reconstructed from memory or from ASTAP's online docs.

Sources used:
- Real production sidecars: `/home/user/astroops/library/**/*.ini`, `*.wcs`
  (mirrored under `/home/user/mnt/astro/Astronomy/library/**`)
- Real ASTAP CLI binary: `/home/user/astap/astap_cli` (version string:
  `ASTAP astrometric solver version CLI-2026.06.29`, `(C) 2018, 2025 by Han Kleijn.
  License MPL 2.0, Webpage: www.hnsky.org`)
- Real DB: `/home/user/astroops/state/catalogue.db` (copied to `/tmp` for read-only
  querying, copy deleted after use)
- Controlled re-run of the real binary against a real production FITS file
  (`/tmp/test_good.fits`, a copy of
  `library/prawn/lights/S/2026-07-28_23-12-39_S_300.00s_100g_1x1_0023_-9.90.fits`)
  to isolate the effect of the `-wcs` flag. Temp files were deleted after the
  investigation; nothing in `~/astroops` was modified.

---

## 1. ASTAP's `.ini` sidecar — exact format

### 1a. Complete real success-case file, verbatim

Path: `/home/user/astroops/library/prawn/lights/S/2026-07-28_23-12-39_S_300.00s_100g_1x1_0023_-9.90.ini`
(216 bytes on disk in some copies, but the version below — the canonical one in
`~/astroops/library` — is the one hexdumped; sizes vary slightly between the
`~/astroops/library` and `~/mnt/astro` mirrors because CMDLINE differs slightly
per invocation, see §1c)

```
PLTSOLVD=T
CRPIX1= 1.9205000000000000E+003
CRPIX2= 1.0805000000000000E+003
CRVAL1= 2.5423046742390622E+002
CRVAL2=-4.0311880588850023E+001
CDELT1= 6.8154932258843713E-004
CDELT2= 6.8151366119530501E-004
CROTA1=-5.8859778367665449E+001
CROTA2=-5.8866887820396883E+001
CD1_1= 3.5245253250848707E-004
CD1_2= 5.8334097357301367E-004
CD2_1=-5.8335417754934037E-004
CD2_2= 3.5236170894630648E-004
CMDLINE=/home/user/astap/astap_cli -f /home/user/mnt/astro/Astronomy/library/prawn/lights/S/2026-07-28_23-12-39_S_300.00s_100g_1x1_0023_-9.90.fits -r 180 -fov 1.4770 -d /home/user/astap -update
```
(file ends with a trailing `\n` after the CMDLINE line — 15 physical lines total
including the final blank one produced by the trailing newline)

Full hexdump (7028... actually 573 bytes, confirmed via `xxd`):

```
00000000: 504c 5453 4f4c 5644 3d54 0a43 5250 4958  PLTSOLVD=T.CRPIX
00000010: 313d 2031 2e39 3230 3530 3030 3030 3030  1= 1.92050000000
00000020: 3030 3030 3045 2b30 3033 0a43 5250 4958  00000E+003.CRPIX
00000030: 323d 2031 2e30 3830 3530 3030 3030 3030  2= 1.08050000000
00000040: 3030 3030 3045 2b30 3033 0a43 5256 414c  00000E+003.CRVAL
00000050: 313d 2032 2e35 3432 3330 3436 3734 3233  1= 2.54230467423
00000060: 3930 3632 3245 2b30 3032 0a43 5256 414c  90622E+002.CRVAL
00000070: 323d 2d34 2e30 3331 3138 3830 3538 3838  2=-4.03118805888
00000080: 3530 3032 3345 2b30 3031 0a43 4445 4c54  50023E+001.CDELT
00000090: 313d 2036 2e38 3135 3439 3332 3235 3838  1= 6.81549322588
000000a0: 3433 3731 3345 2d30 3034 0a43 4445 4c54  43713E-004.CDELT
000000b0: 323d 2036 2e38 3135 3133 3636 3131 3935  2= 6.81513661195
000000c0: 3330 3530 3145 2d30 3034 0a43 524f 5441  30501E-004.CROTA
000000d0: 313d 2d35 2e38 3835 3937 3738 3336 3736  1=-5.88597783676
000000e0: 3635 3434 3945 2b30 3031 0a43 524f 5441  65449E+001.CROTA
000000f0: 323d 2d35 2e38 3836 3638 3837 3832 3033  2=-5.88668878203
00000100: 3936 3838 3345 2b30 3031 0a43 4431 5f31  96883E+001.CD1_1
00000110: 3d20 332e 3532 3435 3235 3332 3530 3834  = 3.524525325084
00000120: 3837 3037 452d 3030 340a 4344 315f 323d  8707E-004.CD1_2=
00000130: 2035 2e38 3333 3430 3937 3335 3733 3031   5.8334097357301
00000140: 3336 3745 2d30 3034 0a43 4432 5f31 3d2d  367E-004.CD2_1=-
00000150: 352e 3833 3335 3431 3737 3534 3933 3430  5.83354177549340
00000160: 3337 452d 3030 340a 4344 325f 323d 2033  37E-004.CD2_2= 3
00000170: 2e35 3233 3631 3730 3839 3436 3330 3634  .523617089463064
00000180: 3845 2d30 3034 0a43 4d44 4c49 4e45 3d2f  8E-004.CMDLINE=/
00000190: 5573 6572 732f 6e72 662f 6173 7461 702f  Users/nrf/astap/
000001a0: 6173 7461 705f 636c 6920 2d66 202f 5573  astap_cli -f /Us
000001b0: 6572 732f 6e72 662f 6d6e 742f 6173 7472  ers/nrf/mnt/astr
000001c0: 6f2f 4173 7472 6f6e 6f6d 792f 6c69 6272  o/Astronomy/libr
000001d0: 6172 792f 7072 6177 6e2f 6c69 6768 7473  ary/prawn/lights
000001e0: 2f53 2f32 3032 362d 3037 2d32 385f 3233  /S/2026-07-28_23
000001f0: 2d31 322d 3339 5f53 5f33 3030 2e30 3073  -12-39_S_300.00s
00000200: 5f31 3030 675f 3178 315f 3030 3233 5f2d  _100g_1x1_0023_-
00000210: 392e 3930 2e66 6974 7320 2d72 2031 3830  9.90.fits -r 180
00000220: 202d 666f 7620 312e 3437 3730 202d 6420   -fov 1.4770 -d 
00000230: 2f55 7365 7273 2f6e 7266 2f61 7374 6170  /home/user/astap
00000240: 202d 7570 6461 7465 0a                    -update.
```

### 1b. Exact key list and order (success case)

```
1  PLTSOLVD=T
2  CRPIX1=<value>
3  CRPIX2=<value>
4  CRVAL1=<value>
5  CRVAL2=<value>
6  CDELT1=<value>
7  CDELT2=<value>
8  CROTA1=<value>
9  CROTA2=<value>
10 CD1_1=<value>
11 CD1_2=<value>
12 CD2_1=<value>
13 CD2_2=<value>
14 CMDLINE=<full invoked command line, verbatim>
```
No `[section]` headers. No blank lines between keys. Key order is fixed and always
in this order (verified across all 28 real `PLTSOLVD=T` files on this machine).

### 1c. Value formatting

- All CRPIX/CRVAL/CDELT/CROTA/CD* values use scientific-notation with **exactly
  16 significant digits of mantissa** (1 digit before the decimal point, 16 digits
  after — actually printed as `D.DDDDDDDDDDDDDDDDE±NNN`, i.e.
  1 + 16 = 17 total mantissa digits) and a **4-digit signed exponent**, e.g.:
  - `CRPIX1= 1.9205000000000000E+003` (positive value → leading space, no `+` sign
    on the value itself, only on the exponent)
  - `CRVAL2=-4.0311880588850023E+001` (negative value → `-` directly after `=`,
    no space)
  - `CDELT1= 6.8154932258843713E-004` (exponent is `E-004`, i.e. `E` + sign +
    3-digit zero-padded exponent, **not** `E-4` or `E-04`)
- Positive values get a single leading space where a negative value would have its
  `-` sign (column alignment), e.g. `CRPIX1= 1.92...` vs `CRVAL2=-4.03...`.
- `PLTSOLVD` is a bare `T` or `F` (FITS boolean convention), no quotes.
- `CMDLINE` is the literal command line ASTAP was invoked with, copied verbatim —
  this is the ground truth for what arguments were actually passed for a given
  frame (see confirmed real invocations below, §3).

### 1d. Line endings / trailing newline

- **LF only** (`0x0a`), no CR anywhere in either file examined.
- File **ends with a trailing newline** after the last key (`CMDLINE=...\n`).

### 1e. Failure case — exact format, TWO distinct real ERROR strings found

46 `.ini` files exist under `~/astroops/library`, 83 under `~/mnt/astro/...library`
(mirrors of the same files — see note in §5). Across all of them:
`PLTSOLVD=F` → 101 rows (mirror-inclusive), `PLTSOLVD=T` → 28 rows.

**Every distinct `ERROR=` string observed on this machine (exhaustive grep over
all real `.ini` files):**
```
ERROR=No star database found.
ERROR=Not enough stars.
```

Failure-case file structure (verbatim, hexdumped), example 1 — "No star database found":

Path: `/home/user/astroops/library/prawn/lights/S/2026-07-28_21-56-37_S_300.00s_100g_1x1_0019_-9.90.ini`

```
00000000: 0a50 4c54 534f 4c56 443d 460a 434d 444c  .PLTSOLVD=F.CMDL
00000010: 494e 453d 2f55 7365 7273 2f6e 7266 2f61  INE=/home/user/a
00000020: 7374 6170 2f61 7374 6170 5f63 6c69 202d  stap/astap_cli -
00000030: 6620 2f55 7365 7273 2f6e 7266 2f6d 6e74  f /home/user/mnt
00000040: 2f61 7374 726f 2f41 7374 726f 6e6f 6d79  /astro/Astronomy
00000050: 2f6c 6962 7261 7279 2f70 7261 776e 2f6c  /library/prawn/l
00000060: 6967 6874 732f 532f 3230 3236 2d30 372d  ights/S/2026-07-
00000070: 3238 5f32 312d 3536 2d33 375f 535f 3330  28_21-56-37_S_30
00000080: 302e 3030 735f 3130 3067 5f31 7831 5f30  0.00s_100g_1x1_0
00000090: 3031 395f 2d39 2e39 302e 6669 7473 202d  019_-9.90.fits -
000000a0: 7220 3138 3020 2d66 6f76 2031 2e34 3737  r 180 -fov 1.477
000000b0: 3020 2d75 7064 6174 650a 4552 524f 523d  0 -update.ERROR=
000000c0: 4e6f 2073 7461 7220 6461 7461 6261 7365  No star database
000000d0: 2066 6f75 6e64 2e0a                       found..
```

Decoded:
```
                                    <-- IMPORTANT: file starts with a literal blank line (\n at byte 0)!
PLTSOLVD=F
CMDLINE=/home/user/astap/astap_cli -f /home/user/mnt/astro/Astronomy/library/prawn/lights/S/2026-07-28_21-56-37_S_300.00s_100g_1x1_0019_-9.90.fits -r 180 -fov 1.4770 -update
ERROR=No star database found.
```

Failure-case example 2 — "Not enough stars" (path:
`/home/user/astroops/library/_probe/lights/L/2026-07-30_19-58-21_L_15.00s_100g_1x1_0000_-9.90.ini`), same structure:
```
                                    <-- blank first line again
PLTSOLVD=F
CMDLINE=/home/user/astap/astap_cli -f /home/user/mnt/astro/Astronomy/library/_probe/lights/L/2026-07-30_19-58-21_L_15.00s_100g_1x1_0000_-9.90.fits -r 180 -fov 1.4770 -d /home/user/astap -update
ERROR=Not enough stars.
```

**Key differences from the success case:**
- **Key order for failure**: `PLTSOLVD=F`, then `CMDLINE=...`, then `ERROR=<message>`
  (note: `CMDLINE` comes *before* `ERROR`, opposite of what you'd assume; and none
  of the CRPIX/CRVAL/CD keys are present at all when the solve fails).
- **The file begins with one blank line** (a bare `\n` as the very first byte)
  before `PLTSOLVD=F` in both real failure files examined — this looks like a
  genuine ASTAP quirk (an extra `writeln` before the first field on the failure
  path), not a copy artifact; verified independently in two unrelated failure
  files with different content afterward.
- `ERROR=` message text ends with a period and is a single line, LF-terminated.
- File still ends with a **trailing newline**.
- I independently reproduced this failure locally by running the real binary
  against a fake `-d` path — got the identical structure and byte-for-byte
  identical `ERROR=No star database found.` string (see §3 for the exit code
  from that run — it was `1`).

---

## 2. ASTAP's `.wcs` sidecar

**Two distinct on-disk formats exist, controlled by the `-wcs` CLI flag** — this
was independently confirmed by re-running the real `astap_cli` binary on a real
production FITS file both with and without `-wcs`. All real production `.wcs`
files found in `~/astroops/library` were produced **without** `-wcs` (their
recorded `CMDLINE`/`COMMENT cmdline:` never includes `-wcs`), i.e. every real
`.wcs` file on this machine is the ASTAP **"text style" default**, not the
"Astrometry.net-style" binary FITS block — but the plan should support both since
`-wcs` is a documented flag.

### 2a. Default format (no `-wcs` flag) — what every real file on disk actually is

This is **not** a padded, no-newline FITS block. It is a FITS-header-*styled*
**text** file: 80-column-formatted cards, each followed by a single `0x0a` (LF),
**not** padded to a 2880-byte boundary, and the file is simply as long as its
content requires (no padding block at the end).

Real file: `/home/user/astroops/library/prawn/lights/S/2026-07-28_23-12-39_S_300.00s_100g_1x1_0023_-9.90.wcs`
— 7028 bytes, 87 lines (`wc -l` = 87), confirmed formula:
`85 cards × 80 chars + 1 card of 84 chars + 1 card of 57 chars + 87 newlines = 7028`.
So **most** cards are exactly 80 characters (standard FITS card width) followed by
`\n`, but at least one COMMENT card in the real file is 84 chars (exceeds 80,
not truncated) and the final COMMENT card is left un-padded at 57 chars (no
trailing spaces to reach 80). This is a real deviation from the FITS standard's
strict fixed-width card rule — important for a "byte-compatible" implementation.

First 400 bytes, hexdump:
```
00000000: 5349 4d50 4c45 2020 3d20 2020 2020 2020  SIMPLE  =       
00000010: 2020 2020 2020 2020 2020 2020 2054 202f               T /
00000020: 2043 2320 4649 5453 2020 2020 2020 2020   C# FITS        
00000030: 2020 2020 2020 2020 2020 2020 2020 2020                  
00000040: 2020 2020 2020 2020 2020 2020 2020 2020                  
00000050: 0a42 4954 5049 5820 203d 2020 2020 2020  .BITPIX  =      
00000060: 2020 2020 2020 2020 2020 2020 2020 3820                8 
00000070: 2020 2020 2020 2020 2020 2020 2020 2020                  
00000080: 2020 2020 2020 2020 2020 2020 2020 2020                  
00000090: 2020 2020 2020 2020 2020 2020 2020 2020                  
000000a0: 200a 4e41 5849 5320 2020 3d20 2020 2020   .NAXIS   =     
000000b0: 2020 2020 2020 2020 2020 2020 2020 2030                 0
000000c0: 202f 2044 696d 656e 7369 6f6e 616c 6974   / Dimensionalit
000000d0: 7920 2020 2020 2020 2020 2020 2020 2020  y               
000000e0: 2020 2020 2020 2020 2020 2020 2020 2020                  
000000f0: 2020 0a42 5a45 524f 2020 203d 2020 2020    .BZERO   =    
00000100: 2020 2020 2020 2020 2020 2020 3332 3736              3276
00000110: 3820 2020 2020 2020 2020 2020 2020 2020  8               
00000120: 2020 2020 2020 2020 2020 2020 2020 2020                  
00000130: 2020 2020 2020 2020 2020 2020 2020 2020                  
00000140: 2020 200a 4558 5445 4e44 2020 3d20 2020     .EXTEND  =   
00000150: 2020 2020 2020 2020 2020 2020 2020 2020                  
00000160: 2054 202f 2045 7874 656e 7369 6f6e 7320   T / Extensions 
00000170: 6172 6520 7065 726d 6974 7465 6420 2020  are permitted   
00000180: 2020 2020 2020 2020 2020 2020 2020 2020                  
```

Full decoded card list (all 87 lines, in order, verbatim) — this file is a
**complete copy of the original capture-software FITS header** (N.I.N.A.
metadata: OBJECT, camera, focuser, weather, mount pointing, etc.) with the WCS
solution keywords and solver `COMMENT`s appended at the end:
```
SIMPLE  =                    T / C# FITS
BITPIX  =                    8
NAXIS   =                    0 / Dimensionality
BZERO   =                32768
EXTEND  =                    T / Extensions are permitted
IMAGETYP= 'LIGHT'              / Type of exposure
EXPOSURE=                300.0 / [s] Exposure duration
EXPTIME =                300.0 / [s] Exposure duration
DATE-LOC= '2026-07-28T23:12:39.9620845' / Time of observation (local)
DATE-OBS= '2026-07-28T11:12:39.9620845' / Time of observation (UTC)
MJD-OBS =       61249.46712919 / Modified Julian Date of observation
DATE-AVG= '2026-07-28T11:15:10.0100053' / Averaged midpoint time (UTC)
MJD-AVG =     61249.4688658565 / Modified Julian Date of averaged midpoint
XBINNING=                    1 / X axis binning factor
YBINNING=                    1 / Y axis binning factor
GAIN    =                  100 / Sensor gain
OFFSET  =                    0 / Sensor gain offset
XPIXSZ  =                  2.9 / [um] Pixel X axis size
YPIXSZ  =                  2.9 / [um] Pixel Y axis size
INSTRUME= 'ATR585M'            / Imaging instrument name
CAMERAID= 'ToupTek_\\?\usb#vid_0547&pid_157c#5&2ec8f72a&0&15' / Imaging instrume
SET-TEMP=                -10.0 / [degC] CCD temperature setpoint
CCD-TEMP=                 -9.9 / [degC] CCD temperature
READOUTM= 'High Dynamic Range' / Sensor readout mode
USBLIMIT=                    2 / Camera-specific USB setting
TELESCOP= 'SV555'              / Name of telescope
FOCALLEN=                243.0 / [mm] Focal length
FOCRATIO=                  4.5 / Focal ratio
RA      =     253.841369932903 / [deg] RA of telescope
DEC     =    -41.1207840782912 / [deg] Declination of telescope
CENTALT =              61.2525 / [deg] Altitude of telescope
CENTAZ  =     252.016388888889 / [deg] Azimuth of telescope
AIRMASS =     1.14017044777361 / Airmass at frame center (Gueymard 1993)
PIERSIDE= 'East'               / Telescope pointing state
SITEELEV=     000.000000000000 / [m] Observation site elevation
SITELAT =           -00.000000 / [deg] Observation site latitude
SITELONG=           000.000000 / [deg] Observation site longitude
FWHEEL  = 'ASCOM.ToupTek.FilterWheel' / Filter Wheel name
FILTER  = 'S'                  / Active filter name
OBJECT  = 'Prawn Nebula (IC 4628)' / Name of the object of interest
OBJCTRA = '16 57 00'           / [H M S] RA of imaged object
OBJCTDEC= '-40 20 00'          / [D M S] Declination of imaged object
OBJCTROT=                22.35 / [deg] planned rotation of imaged object
FOCNAME = 'ASCOM.ToupTek.AAF'  / Focusing equipment name
FOCPOS  =                 3592 / [step] Focuser position
FOCUSPOS=                 3592 / [step] Focuser position
FOCUSSZ =                100.0 / [um] Focuser step size
FOCTEMP =                  2.1 / [degC] Focuser temperature
FOCUSTEM=                  2.1 / [degC] Focuser temperature
CLOUDCVR=                  0.0 / [percent] Cloud cover
DEWPOINT=    0.603645093036094 / [degC] Dew point
HUMIDITY=                 95.0 / [percent] Relative humidity
PRESSURE=      991.97766026377 / [hPa] Air pressure
AMBTEMP =     1.30000000000001 / [degC] Ambient air temperature
WINDDIR =                137.0 / [deg] Wind direction: 0=N, 180=S, 90=E, 270=W
WINDGUST=                3.636 / [kph] Wind gust
WINDSPD =                3.492 / [kph] Wind speed
ROWORDER= 'TOP-DOWN'           / FITS Image Orientation
EQUINOX =               2000.0 / Equinox of coordinates
SWCREATE= 'N.I.N.A. 3.2.0.9001 (x64)' / Software that created this file
CTYPE1  = 'RA---TAN'           / first parameter RA,    projection TANgential
CTYPE2  = 'DEC--TAN'           / second parameter DEC,  projection TANgential
CUNIT1  = 'deg     '           / Unit of coordinates
CRPIX1  =  1.920500000000E+003 / X of reference pixel
CRPIX2  =  1.080500000000E+003 / Y of reference pixel
CRVAL1  =  2.542304674239E+002 / RA of reference pixel (deg)
CRVAL2  = -4.031188058885E+001 / DEC of reference pixel (deg)
CDELT1  =  6.815493225884E-004 / X pixel size (deg)
CDELT2  =  6.815136611953E-004 / Y pixel size (deg)
CROTA1  = -5.885977836767E+001 / Image twist of X axis        (deg)
CROTA2  = -5.886688782040E+001 / Image twist of Y axis        (deg)
CD1_1   =  3.524525325085E-004 / CD matrix to convert (x,y) to (Ra, Dec)
CD1_2   =  5.833409735730E-004 / CD matrix to convert (x,y) to (Ra, Dec)
CD2_1   = -5.833541775493E-004 / CD matrix to convert (x,y) to (Ra, Dec)
CD2_2   =  3.523617089463E-004 / CD matrix to convert (x,y) to (Ra, Dec)
PLTSOLVD=                    T / Astrometric solved by ASTAP_CLI v2026.06.29.
COMMENT 7 Solved in 0.1 sec. Offset was 0.0". Mount offset RA=-1061.7", DEC=-2912.1"
COMMENT cmdline:/home/user/astap/astap_cli -f /home/user/mnt/astro/Astronomy/inb
COMMENT ox/capture/2026-07-28_23-12-39_S_300.00s_100g_1x1_0023_-9.90.fits -ra 16
COMMENT .950000 -spd 49.666667 -r 15 -fov 1.4770 -d /home/user/astap -update
COMMENT cmdline:/home/user/astap/astap_cli -f /home/user/mnt/astro/Astronomy/lib
COMMENT rary/prawn/lights/S/2026-07-28_23-12-39_S_300.00s_100g_1x1_0023_-9.90.fi
COMMENT ts -r 180 -fov 1.4770 -d /home/user/astap -update
COMMENT cmdline:/home/user/astap/astap_cli -f /home/user/mnt/astro/Astronomy/lib
COMMENT rary/prawn/lights/S/2026-07-28_23-12-39_S_300.00s_100g_1x1_0023_-9.90.fi
COMMENT ts -r 180 -fov 1.4770 -d /home/user/astap -update
END
```

Note the WCS numeric formatting here is **different** from the `.ini` file's:
scientific notation with only **12 mantissa digits** (`E.g. 1.920500000000E+003`,
i.e. `D.DDDDDDDDDDDDE±NNN` = 1+12 = 13 mantissa digits) vs 16+1=17 in the `.ini`.
Also note **this is real, independent evidence that `-ra`/`-spd` are used
internally by ASTAP's own retry logic**: one of the embedded `COMMENT cmdline:`
lines shows ASTAP re-invoking itself (or logging a prior invocation) as
`-ra 16.950000 -spd 49.666667 -r 15 -fov 1.4770` — where `OBJCTRA='16 57 00'`
(16h57m00s = 16.95 h) and `OBJCTDEC='-40 20 00'` (-40.333333°) →
`spd = dec + 90 = 49.666667`. This is a **verbatim, ground-truth confirmation**
that `-ra` is in **hours** and `-spd = dec_deg + 90`.

### 2b. `-wcs` flag format — true binary-style FITS header block

Reproduced locally by running the real `astap_cli` with `-wcs` on the same
source FITS file. Result:
- **8640 bytes exactly** = 3 × 2880 (a real FITS-block-padded file).
- **Zero LF bytes anywhere** in the file (`newline count == 0` confirmed via
  Python byte-count) — cards are concatenated directly, no line breaks.
- **108 cards of exactly 80 bytes each** (8640 / 80 = 108), all content
  identical to §2a's card list but with all inter-card newlines removed and
  every card force-padded to exactly 80 bytes (even the previously-84-byte
  COMMENT card is truncated/repadded to 80, and the previously-57-byte final
  COMMENT card is space-padded to 80).
- `END` card found at **card index 88** (byte offset 7040), exactly matching
  real FITS convention; **cards 89–107 (19 cards) are entirely blank
  (80 spaces each)** — i.e. genuine FITS-block padding to the 2880-byte
  boundary, confirmed byte-for-byte (`b'END' + 77*b' '`, then 19 more
  all-space 80-byte cards).

So: **`-wcs` produces a genuinely FITS-spec-compliant header block; the default
(no `-wcs`) format is a look-alike text file with embedded newlines and no block
padding.** Every real production `.wcs` sidecar found on this machine uses the
**default (no `-wcs`)** text-style format — grep of all real `.ini`/`.wcs`
`CMDLINE`/`COMMENT cmdline:` strings on this machine found zero occurrences of
`-wcs`.

---

## 3. ASTAP's CLI surface

Binary location: `/home/user/astap/astap_cli` (not on `$PATH`; `which astap` finds
nothing; there is no `astap` shim, only `astap_cli`). Star database present:
`/home/user/astap/d50_*.1476` files (the "D50" database).

### 3a. Complete verbatim help text (`astap_cli --help`, `-h`, and no-args all
identical, all exit 0)

```
ASTAP astrometric solver version CLI-2026.06.29
(C) 2018, 2025 by Han Kleijn. License MPL 2.0, Webpage: www.hnsky.org
Usage:
-f  filename {fits, tiff, png, pbm, jpg files}
-r  radius_area_to_search[degrees]
-fov diameter_field[degrees] {enter zero for auto}
-ra  right_ascension[hours]
-spd south_pole_distance[degrees]
-s  max_number_of_stars  {default 500}
-t  quad_tolerance  {default 0.007}
-m  minimum_star_size["]  {default 1.5}
-z  downsample_factor[0,1,2,3,4,..] {Downsample prior to solving. Specify 0 for auto selection}
-check {Apply check pattern filter prior to solving. Use for raw OSC images only when binning is 1x1}
-d  path {specify a path to the star database}
-D  abbreviation[d80,d50,...] {Specify a star database}
-o  file {Name the output files with this base path & file name.}
-sip {Add SIP (Simple Image Polynomial) coefficients}
-speed mode[auto/slow] {Slow is forcing more area overlap while searching to improve detection}
-wcs  {Write a .wcs file  in similar format as Astrometry.net. Else text style}
-log  {Write the solver log to a .log text file.}
-update  {Add the solution to the input fits file header. Jpeg, png, tiff will be written as fits}
-progress   {Log all progress steps and messages}

Analyse options:
-analyse snr_min {Analyse only and report median HFD and number of stars used}
-extract snr_min {Export info of all detectable stars to a .csv file}
-extract2 snr_min {Solve and export info of all detectable stars to a .csv file including ra, dec}

Preference will be given to the command-line values. CSV files are written with a dot as decimal seperator.
Solver result will be written to filename.ini and filename.wcs.
```

**No man page exists** (`man astap` fails); **no bundled README/docs** in
`/home/user/astap/` beyond the star database files and the binary itself; the
`strings` search for exit-code documentation inside the binary turned up nothing
usable (no help text embedded beyond the above). Exit codes below are therefore
**empirically observed**, not documented by ASTAP itself.

### 3b. Flag semantics (confirmed, some empirically via live re-run of the binary)

| Flag | Meaning | Units / notes (confirmed) |
|---|---|---|
| `-f` | input filename | fits/tiff/png/pbm/jpg |
| `-r` | search radius | degrees (real invocations use `-r 180`, i.e. blind/all-sky, and `-r 15` for a narrow retry) |
| `-ra` | right ascension of estimated center | **hours** — confirmed from real embedded `COMMENT cmdline:` in a real `.wcs`: `-ra 16.950000` corresponds to `OBJCTRA='16 57 00'` = 16h57m00s = 16.95 h |
| `-spd` | south polar distance | degrees, **= dec_deg + 90** — confirmed from same real comment: `-spd 49.666667` corresponds to `OBJCTDEC='-40 20 00'` = -40.333333°, and -40.333333 + 90 = 49.666667 exactly |
| `-fov` | field diameter | degrees; real invocations use `-fov 1.4770`; "enter zero for auto" |
| `-d` | path to star database directory | e.g. real `CMDLINE` uses `-d /home/user/astap`; confirmed: omitting `-d` when no DB is on a default search path produces `ERROR=No star database found.` and exit code 1 (reproduced live) |
| `-D` | named DB abbreviation (`d80`,`d50`,...) | alternative to `-d` |
| `-o` | output base path/filename | "Name the output files with this base path & file name" — not exercised live in this investigation |
| `-update` | write solution into the FITS header in-place | real invocations always use this; confirmed present in every real `CMDLINE` examined |
| `-wcs` | switch `.wcs` sidecar from default "text style" to FITS-block "Astrometry.net" style | confirmed live (§2b) — real production runs never pass this flag |
| `-z` | downsample factor | integer 0 (auto) upward |
| `-s` | max stars to use | default 500 |
| `-t` | quad match tolerance | default 0.007 |
| `-m` | minimum star size | arcsec, default 1.5 |
| `-log` | write `.log` solver log | not exercised |
| `-sip` | add SIP polynomial coefficients | not exercised |
| `-speed` | `auto`/`slow` | not exercised |
| `-check` | checker-pattern filter for OSC raw images at 1x1 binning | not exercised |
| `-progress` | verbose progress logging | not exercised |
| `-analyse` / `-extract` / `-extract2` | analysis-only / star-CSV-export modes | not exercised |

Real production invocations recorded verbatim in this investigation (from actual
`.ini`/`.wcs` `CMDLINE`/`COMMENT cmdline:` fields — not constructed):
```
/home/user/astap/astap_cli -f <path>.fits -r 180 -fov 1.4770 -d /home/user/astap -update
/home/user/astap/astap_cli -f <path>.fits -ra 16.950000 -spd 49.666667 -r 15 -fov 1.4770 -d /home/user/astap -update
```
(the second is a narrow-radius retry seeded with the target's nominal RA/Dec after
a wider blind `-r 180` search; note this second real example omits `-d` in one
recorded instance and includes it in another — both forms occur in real captured
CMDLINEs on this machine.)

### 3c. Exit codes (empirically observed — no documented list exists in the help
text, man page, or binary strings)

| Scenario | Exit code | Evidence |
|---|---|---|
| Successful solve | `0` | live re-run: real fits, prints `Solution found: ...`, `$?` = 0 |
| `--help`/`-h`/no args | `0` | live run, `$?` = 0 |
| Input file does not exist | `1` | live run: `Error, accessing the file!`, `$?` = 1 |
| Star database not found (`-d` points nowhere) | `1` | live run: `Error, no star database found at <path>/ ! Download and install a star database.`, `$?` = 1; also produces `.ini` with `ERROR=No star database found.` |
| Not enough stars detected (real historical failure, not reproduced live) | presumed `1` (same family of `.ini` `ERROR=` failures as above; not independently re-run in this investigation — the "No star database found" case *was* independently reproduced and returned 1) |

---

## 4. The AstroOps solve database

Path: `/home/user/astroops/state/catalogue.db` (plain file, no `-wal`/`-shm`
sidecars present at query time). Queried via a `/tmp` copy, read-only, then the
copy was deleted.

### 4a. Complete `CREATE TABLE` DDL (`.schema` output, verbatim)

```sql
CREATE TABLE frame (
  id INTEGER PRIMARY KEY, rig TEXT NOT NULL, captured_at TEXT NOT NULL,
  image_type TEXT, exposure_s REAL, filt TEXT, gain REAL, binning INTEGER,
  ccd_temp REAL, set_temp REAL, rotation REAL, focuser_pos INTEGER,
  focuser_temp REAL, ambient_c REAL, dew_c REAL, naxis1 INTEGER,
  naxis2 INTEGER, bitpix INTEGER, focal_len REAL, pixel_um REAL,
  object_name TEXT, target_id INTEGER,
  ra_deg REAL, dec_deg REAL, pointing_src TEXT,
  image_type_eff TEXT, image_type_src TEXT,
  filt_eff TEXT, filt_src TEXT,
  UNIQUE(rig, captured_at));
CREATE INDEX frame_target ON frame(target_id);
CREATE INDEX frame_when ON frame(captured_at);
CREATE TABLE location (
  path TEXT PRIMARY KEY, frame_id INTEGER NOT NULL, tree TEXT,
  mtime REAL, size INTEGER, bytes_expected INTEGER, intact INTEGER DEFAULT 1,
  filed_at REAL);
CREATE INDEX location_frame ON location(frame_id);
CREATE TABLE target (
  id INTEGER PRIMARY KEY, slug TEXT UNIQUE NOT NULL,
  ra_deg REAL, dec_deg REAL, source TEXT);
CREATE TABLE measurement (
  frame_id INTEGER NOT NULL, tool_version TEXT NOT NULL, hfr REAL,
  fwhm_px REAL, roundness REAL, background REAL, bg_noise REAL,
  star_count INTEGER, ra_deg REAL, dec_deg REAL, measured_at TEXT,
  PRIMARY KEY (frame_id, tool_version));
CREATE TABLE grade (
  frame_id INTEGER NOT NULL, criteria_version INTEGER NOT NULL,
  n_transparency REAL, n_focus REAL, n_tracking REAL, n_sky REAL,
  verdict TEXT, reason TEXT, graded_at TEXT,
  PRIMARY KEY (frame_id, criteria_version));
CREATE TABLE context (
  frame_id INTEGER PRIMARY KEY, alt REAL, az REAL, hour_angle REAL,
  moon_sep REAL, moon_alt REAL, moon_illum REAL,
  cloud_pct REAL, sqm REAL, guide_rms REAL);
CREATE TABLE rejected (
  path TEXT PRIMARY KEY, reason TEXT NOT NULL, seen_at TEXT NOT NULL);
```

**IMPORTANT — there is no table/column holding CRVAL/CRPIX/CD (a full WCS
solution).** The `measurement` table (keyed `(frame_id, tool_version)`) is where
ASTAP results live, identified by `tool_version = 'astap/astap+d50'`, but it only
stores the **solved center** (`ra_deg`, `dec_deg`) — no pixel scale, no rotation,
no CD matrix, no CRPIX. `star_count`/`hfr`/`fwhm_px`/etc. columns exist in the
schema but were **NULL/empty in all 3 sampled ASTAP rows** (see §4c) — they
appear to belong to a different `tool_version` (there are also `ingest` (1342
rows) and `grade` (435 rows) values in `tool_version`, which presumably populate
those columns instead). `frame.ra_deg`/`frame.dec_deg`/`pointing_src` is a
separate, frame-level "best known pointing" field (`pointing_src` values:
`commanded` 14594, `solve` 375, empty 1) — distinct from the per-tool
`measurement` rows.

### 4b. Row counts (all tables, exact)

```
frame:        14970
location:      14974
target:          131
measurement:   11272
grade:           466
context:         466
rejected:          2
```

### 4c. The ~9,495 ASTAP-solve claim — verified exactly

```sql
SELECT tool_version, COUNT(*) FROM measurement GROUP BY tool_version ORDER BY 2 DESC;
```
```
astap/astap+d50|9495
ingest|1342
grade|435
```
```sql
SELECT COUNT(*) FROM measurement WHERE tool_version='astap/astap+d50';
```
→ **9495** — the user's ~9,495 figure is exact, not approximate. `tool_version`
is literally the string `astap/astap+d50` (identifies the ASTAP D50 star
database, not an ASTAP program version number like `2026.06.29`).

### 4d. Three complete example rows (`.mode line`, verbatim)

```sql
SELECT * FROM measurement WHERE tool_version='astap/astap+d50' LIMIT 3;
```
```
    frame_id = 3
tool_version = astap/astap+d50
         hfr =
     fwhm_px =
   roundness =
  background =
    bg_noise =
  star_count =
      ra_deg = 83.8606697119768
     dec_deg = -5.39697562746118
 measured_at =

    frame_id = 4
tool_version = astap/astap+d50
         hfr =
     fwhm_px =
   roundness =
  background =
    bg_noise =
  star_count =
      ra_deg = 83.8611530867122
     dec_deg = -5.39736998928681
 measured_at =

    frame_id = 5
tool_version = astap/astap+d50
         hfr =
     fwhm_px =
   roundness =
  background =
    bg_noise =
  star_count =
      ra_deg = 83.8646234769498
     dec_deg = -5.39775445089091
 measured_at =
```
(Note: `hfr`, `fwhm_px`, `roundness`, `background`, `bg_noise`, `star_count`,
`measured_at` are all NULL/empty for every sampled ASTAP row — only `ra_deg`/
`dec_deg` are populated for this `tool_version`.)

Joined example, showing frame path via `location`:
```sql
SELECT f.id, f.rig, f.captured_at, f.naxis1, f.naxis2, f.binning, f.filt,
       f.ra_deg AS frame_ra, f.dec_deg AS frame_dec,
       m.ra_deg AS solve_ra, m.dec_deg AS solve_dec, l.path
FROM measurement m JOIN frame f ON f.id = m.frame_id
JOIN location l ON l.frame_id = f.id
WHERE m.tool_version='astap/astap+d50' LIMIT 3;
```
```
id=3  rig=DWARFIII  captured_at=2024-12-28T23:41:12.383000+00:00
  naxis1=3856 naxis2=2180 binning=1 filt=(empty)
  frame_ra=83.82208 frame_dec=-5.39111
  solve_ra=83.8606697119768 solve_dec=-5.39697562746118
  path=/home/user/astroops/archive/fits/DWARFIII/M_42/2024/12/28/60g/5985.0s/Light/stacked-16_M 42_15s60_Astro_20241228-214221081.fits

id=4  rig=DWARFIII  captured_at=2024-12-28T21:42:35.139000+00:00
  naxis1=3856 naxis2=2180 binning=1 filt=(empty)
  frame_ra=83.82208 frame_dec=-5.39111
  solve_ra=83.8611530867122 solve_dec=-5.39736998928681
  path=/home/user/astroops/archive/fits/DWARFIII/M_42/2024/12/28/60g/15.0s/Light/M 42_15s60_Astro_20241228-214235140_22C.fits

id=5  rig=DWARFIII  captured_at=2024-12-28T21:42:50.140000+00:00
  naxis1=3856 naxis2=2180 binning=1 filt=(empty)
  frame_ra=83.82208 frame_dec=-5.39111
  solve_ra=83.8646234769498 solve_dec=-5.39775445089091
  path=/home/user/astroops/archive/fits/DWARFIII/M_42/2024/12/28/60g/15.0s/Light/M 42_15s60_Astro_20241228-214250140_22C.fits
```

---

## 5. Frame diversity

### 5a. Distinct image dimensions (naxis1 x naxis2, all combos, from `frame` table, 14970 rows)

```sql
SELECT naxis1, naxis2, COUNT(*) FROM frame GROUP BY naxis1, naxis2 ORDER BY 3 DESC;
```
```
3856 x 2180 : 8565   (dominant — DWARFIII, binning 1)
3840 x 2160 : 3938   (ATR585M, binning 1)
4144 x 2822 : 1565   (SVBONY SV405CC)
2072 x 1410 :  891   (binning-2 variant of one of the above, exactly matches 891 binning=2 rows)
+ 11 further one-off odd dimensions (3138x1890, 3392x1920, 3480x1950, 3508x1620,
  3524x1998, 3572x1782, 3612x1930, 3616x1966, 3626x1948, 3632x1950, 3844x2090),
  each with exactly 1 row — almost certainly stacked/cropped master frames rather
  than raw camera-native captures.
```

### 5b. Binning

```
1x1: 14079
2x2:   891   (all 891 are exactly the 2072x1410 dimension bucket above)
```

### 5c. Filters (`filt_eff`, effective/resolved filter column)

```
OSC:      10066
L:         1339
S:         1084
O:          678
H:          623
Duo-Band:   330
R:          291
G:          281
B:          276
(empty):      2
```

### 5d. Declination range

```sql
SELECT MIN(dec_deg), MAX(dec_deg), COUNT(*) FROM frame WHERE dec_deg IS NOT NULL;
```
Frame-level (`frame.dec_deg`, "best known pointing" — commanded or solved):
`-90.0` to `24.1167`, 14969 non-null rows.

ASTAP-solved-only declination range (`measurement.dec_deg` where
`tool_version='astap/astap+d50'`):
`-89.9747221259041` to `24.2182670876749`.

### 5e. Rigs (cameras/telescopes)

```
DWARFIII:            8576
ATR585M:             3938
SVBONY SV405CC:      2454
SVBONY CCD SV405CC:     2
```

### 5f. Total FITS frame counts (filesystem, independent of DB)

```
find ~/astroops/library -iname '*.fits' | wc -l   →  1999
find ~/astroops/archive -iname '*.fits' | wc -l   → 12976
```
(`library` = working/active tree with `.ini`/`.wcs` sidecars sitting next to the
FITS files; `archive` = long-term storage tree without sidecars, referenced only
by DB `location.path`.) DB `location` table row count (all catalogued frame
files across every tree) = **14974**, close to but not identical to `frame` count
(14970) — a handful of frames apparently have duplicate/multiple `location`
rows (e.g. a stacked master alongside its raw source, both pointing at the same
`frame_id`).

### 5g. Representative test-sample recommendation

A representative sample for exercising an ASTAP-compatible solver should span:
at least the two dominant dimension/binning buckets (3856x2180 @ 1x1 and
3840x2160 @ 1x1, which alone cover 12503 of 14970 frames = 83.5%), the 2x2
binning bucket (2072x1410, 891 frames), a broadband filter (OSC, 10066 frames)
and at least one narrowband filter (H/O/S), and declinations spanning the full
observed range down to the pole (-90° to +24°) since real solves exist at
`dec ≈ -89.97°` (near-pole) — a case worth stress-testing given `-spd` (south
polar distance) becomes numerically small/degenerate there.
