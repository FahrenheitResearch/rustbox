# Migration Notes

This file tracks the first real vertical slice that replaced the original scaffold.

## wx-fetch

- Upstream repo: `rusbie`
- Commit: `bd13520b906993d58b1b29d27aaa8aae877a9338`
- Source files:
  - `upstream/rusbie/crates/herbie-core/src/idx.rs`
  - `upstream/rusbie/crates/herbie-core/src/sources.rs`
- Adapted into:
  - [crates/wx-fetch/src/lib.rs](/C:/Users/drew/codex-rustbox/crates/wx-fetch/src/lib.rs)
- What was adapted:
  - `.idx` line parsing shape
  - HRRR AWS URL construction pattern
  - variable/level subset selection semantics
- What was intentionally left out:
  - network clients
  - regex-heavy search surface
  - caching and multi-source fallback

## wx-grib

- Upstream repo: `cfrust`
- Commit: `d326eca4615df9cb3b081181e5e56a9026bb7c3f`
- Source files:
  - `upstream/cfrust/crates/grib-core/src/grib2/parser.rs`
  - `upstream/cfrust/crates/grib-core/src/grib2/unpack.rs`
  - `upstream/cfrust/crates/grib-core/src/grib2/tables.rs`
- Adapted into:
  - [crates/wx-grib/src/lib.rs](/C:/Users/drew/codex-rustbox/crates/wx-grib/src/lib.rs)
- What was adapted:
  - canonical wrapper API that reads a planned subset range and converts one GRIB2 message into `wx-types`
- What was intentionally left out:
  - full parser vendoring
  - JPEG2000/PNG/CCSDS feature plumbing in `rustbox`
  - multi-message search APIs

## wx-thermo

- Upstream repo: `sharprs`
- Commit: `16cf0757304eb690d0208c304e32a4676178f00a`
- Source files:
  - `upstream/sharprs/src/profile.rs`
  - `upstream/sharprs/src/params/cape.rs`
- Adapted into:
  - [crates/wx-thermo/src/lib.rs](/C:/Users/drew/codex-rustbox/crates/wx-thermo/src/lib.rs)
- What was adapted:
  - conversion from `wx-types::SoundingProfile` into SHARP-style profiles
  - real SBCAPE, MLCAPE, MUCAPE, CIN calculations through `parcelx`
  - ECAPE adapter seam
- What was intentionally left out:
  - direct ECAPE computation
  - full SHARP profile/render surface
  - SHARPpy parity verification beyond the pinned `sharprs` implementation

## wx-severe

- Upstream repo: `sharprs`
- Commit: `16cf0757304eb690d0208c304e32a4676178f00a`
- Source files:
  - `upstream/sharprs/src/python.rs`
  - `upstream/sharprs/src/params/composites.rs`
- Adapted into:
  - [crates/wx-severe/src/lib.rs](/C:/Users/drew/codex-rustbox/crates/wx-severe/src/lib.rs)
- What was adapted:
  - self-contained Bunkers storm motion, helicity, and bulk shear helpers from the Python binding layer
  - fixed-layer STP
- What was intentionally left out:
  - effective-layer STP with CIN
  - SCP, SHIP, watch typing, and effective inflow calculations
  - the pinned `winds.rs` path, which is not used for this slice because it fails on the checked-in sounding fixture

## wx-render

- Upstream repo: `wrf-rust-plots`
- Commit: `2088e9599dcf7c7e55be5261c935a48c7afbdd60`
- Source files:
  - `upstream/wrf-rust-plots/crates/wrf-render/src/colormaps.rs`
- Adapted into:
  - [crates/wx-render/src/lib.rs](/C:/Users/drew/codex-rustbox/crates/wx-render/src/lib.rs)
- What was adapted:
  - wind palette anchor colors
  - simple native raster overlay approach
- What was intentionally left out:
  - map projection rendering
  - contours, barbs, labels, and legends

## Fixtures

- HRRR GRIB fixture source:
  - `https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.20240401/conus/hrrr.t00z.wrfsfcf00.grib2`
  - byte range `3566749-5484005`
  - rebased `.idx` excerpt kept in [tests/fixtures/hrrr_gust_surface_fragment.idx](/C:/Users/drew/codex-rustbox/tests/fixtures/hrrr_gust_surface_fragment.idx)
- Sounding fixture source:
  - profile levels copied from `upstream/sharprs/tests/verification.rs` sounding 1
  - expected parcel and severe values pinned to the current `rustbox` implementation against `sharprs` commit `16cf0757304eb690d0208c304e32a4676178f00a`
