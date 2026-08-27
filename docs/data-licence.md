# Data licensing — read before distributing an index

**psolve is free software distributed at no charge.** That places it in the same position
as ASTAP, which has shipped Gaia-derived star databases for years on exactly this basis:
CC BY-NC forbids *commercial use*, not distribution. Shipping a Gaia-derived index as a
release asset is therefore fine, and so is anyone using it for their own astrophotography.

**The obligation that survives is the BY.** Free does not exempt anyone from attribution —
the Gaia mission acknowledgement below must travel with any distributed index.

The rest of this document matters if the project ever stops being free, or if someone
forks it into something that is not.

**The code and the data are licensed separately, and only one of them is permissive.**

| artifact | licence |
|---|---|
| psolve source code | MIT (see `Cargo.toml`) |
| Gaia DR3 catalogue data | **CC BY-NC 3.0 IGO** |
| **an index built from Gaia data** | **CC BY-NC 3.0 IGO** — it is a derivative of the data |

Verified 2026-08-13 at <https://www.cosmos.esa.int/web/gaia-users/license>:

> **GAIA DATA LICENSE.** Gaia data are distributed under the CC BY-NC 3.0 IGO license.
> For details and guidelines concerning commercial use of the Gaia data, please see the
> Terms and Conditions for the use of data in the ESA space science archives.

## What this actually constrains

**NC means non-commercial.** A `.psidx` built by `psolve index build` from Gaia shards is a
derivative work of the catalogue, so it inherits those terms. That is fine for the use this
project was written for — a personal observatory, shared with other amateurs — but it means:

- **Publishing a built index as a release asset is permitted non-commercially**, with
  attribution, and the download must carry the licence notice and the acknowledgement below.
- **Bundling an index into a commercial product is not** covered by this licence. ESA's
  Terms and Conditions for the ESA space science archives is the authority; ask them.
- **The MIT licence on psolve's code does not extend to the index.** Someone may take the
  code commercially; they may not take a Gaia-derived index with it. They would need to
  build their own from a source whose licence permits it — which the `--columns` support
  exists to make possible (Tycho-2, a Vizier export, or their own catalogue).

Nothing here is legal advice. It records what ESA's licence page says and what follows
from it for this repository.

## Required attribution

Any distribution of Gaia-derived data must acknowledge the mission. The canonical wording,
which should be checked against the DR3 credits page
(<https://www.cosmos.esa.int/web/gaia-users/credits>) before publishing:

> This work has made use of data from the European Space Agency (ESA) mission Gaia
> (<https://www.cosmos.esa.int/gaia>), processed by the Gaia Data Processing and Analysis
> Consortium (DPAC, <https://www.cosmos.esa.int/web/gaia/dpac/consortium>). Funding for the
> DPAC has been provided by national institutions, in particular the institutions
> participating in the Gaia Multilateral Agreement.

`psolve index info` reports the index `name` and record digest; when you publish an index,
ship this acknowledgement and the CC BY-NC 3.0 IGO notice alongside it.

## Related

ASTAP, which psolve is an alternative to, is **MPL 2.0** — and its star databases are its
own separate matter. Nothing in this repository derives from ASTAP's code or databases.
