# Migration Notes

This file tracks the first real vertical slice that replaced the original scaffold.

## wx-fetch

- Upstream repo: `rusbie`
- Commit: `bd13520b906993d58b1b29d27aaa8aae877a9338`
- Source files:
  - `upstream/rusbie/crates/herbie-core/src/idx.rs`
  - `upstream/rusbie/crates/herbie-core/src/sources.rs`
- Adapted into:
  - [crates/wx-fetch/src/lib.rs](../crates/wx-fetch/src/lib.rs)
- What was adapted:
  - `.idx` line parsing shape
  - HRRR AWS URL construction pattern
  - variable/level subset selection semantics
  - multi-message request planning for rebased offline fixtures
  - explicit source-object vs rebased-fixture byte-range provenance
  - forecast-text normalization into forecast-hour semantics at planning time for analysis and simple hour/range forecasts
  - last-message range bounding through known fixture length
  - ambiguity rejection when a var/level selector matches multiple forecast entries
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
  - [crates/wx-grib/src/lib.rs](../crates/wx-grib/src/lib.rs)
- What was adapted:
  - canonical wrapper API that reads planned subset ranges and converts GRIB2 messages into `wx-types`
  - multi-message decode path for fixture-backed HRRR fragments
  - exact-one-message enforcement for each selected byte range
  - request-vs-message reference/forecast-time consistency checks at decode time
  - decoded variable/level identity validation so request labels must match the actual GRIB payload
  - HRRR-specific surface/pressure column extraction into `SoundingProfile`
  - bundle-consistency checks across decoded surface/pressure fields
  - a fixed regression-tested HRRR demo column at grid point `(1798, 1058)`
- What was intentionally left out:
  - full parser vendoring
  - JPEG2000/PNG/CCSDS feature plumbing in `rustbox`
  - generalized profile extraction beyond the current HRRR surface + pressure fixture path

## wx-grid

- Upstream repo: `metrust-py`
- Commit: `1b8436779832be326f24bd2acd40a908e9f54685`
- Source files:
  - `upstream/metrust-py/crates/wx-math/src/dynamics.rs`
  - `upstream/metrust-py/crates/metrust/src/calc/smooth.rs`
- Adapted into:
  - [crates/wx-grid/src/lib.rs](../crates/wx-grid/src/lib.rs)
- What was adapted:
  - constant-spacing second-order finite-difference gradient, divergence, vorticity, advection, and Petterssen frontogenesis kernels
  - MetPy-style generic window smoothing plus 5-point and 9-point smoothers
  - `Field2D` wrappers that validate grid/run/level compatibility before deriving new fields
  - isobaric temperature-to-theta conversion so pressure-level frontogenesis is computed from potential temperature rather than raw temperature
  - semantic guards for wind-component and theta/temperature field usage in the meteorological wrappers
  - fixture-backed regression tests against the checked-in HRRR 850 mb pressure slice
- What was intentionally left out:
  - geospatial derivative helpers on full lat/lon grids
  - q-vector, deformation, and potential-vorticity diagnostics
  - GPU dispatch and projection-aware special cases

## wx-thermo

- Upstream repo: `sharprs`
- Commit: `16cf0757304eb690d0208c304e32a4676178f00a`
- Source files:
  - `upstream/sharprs/src/profile.rs`
  - `upstream/sharprs/src/params/cape.rs`
- Adapted into:
  - [crates/wx-thermo/src/lib.rs](../crates/wx-thermo/src/lib.rs)
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
  - `upstream/sharprs/src/winds.rs`
  - `upstream/sharprs/src/params/composites.rs`
- Adapted into:
  - [crates/wx-severe/src/lib.rs](../crates/wx-severe/src/lib.rs)
- What was adapted:
  - exact-layer Bunkers storm motion, helicity, and bulk shear helpers from `winds.rs`
  - a narrow endpoint-wind fallback when direct `Profile::interp_wind()` returns non-finite values at the helicity layer bounds on the checked-in fixture
  - fixed-layer STP
- What was intentionally left out:
  - effective-layer STP with CIN
  - SCP, SHIP, watch typing, and effective inflow calculations
  - direct use of the pinned `sharprs::winds::helicity` call path as the public source of truth, because it still fails on the checked-in fixture profile with `NoData { field: "wind" }`

## wx-render

- Upstream repo: `wrf-rust-plots`
- Commit: `2088e9599dcf7c7e55be5261c935a48c7afbdd60`
- Source files:
  - `upstream/wrf-rust-plots/crates/wrf-render/src/colormaps.rs`
- Adapted into:
  - [crates/wx-render/src/lib.rs](../crates/wx-render/src/lib.rs)
- What was adapted:
  - wind palette anchor colors
  - frontogenesis/vorticity diagnostic palettes
  - simple native raster overlay approach
- What was intentionally left out:
  - map projection rendering
  - contours, barbs, labels, and legends

## Fixtures

- HRRR surface fixture source:
  - `https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.20240401/conus/hrrr.t00z.wrfsfcf00.grib2`
  - selected message ranges rebased into [tests/fixtures/hrrr_demo_surface_fragment.grib2](../tests/fixtures/hrrr_demo_surface_fragment.grib2)
  - matching manifest kept in [tests/fixtures/hrrr_demo_surface_fragment.idx](../tests/fixtures/hrrr_demo_surface_fragment.idx)
- HRRR pressure fixture source:
  - `https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.20240401/conus/hrrr.t00z.wrfprsf00.grib2`
  - selected message ranges rebased into [tests/fixtures/hrrr_demo_pressure_fragment.grib2](../tests/fixtures/hrrr_demo_pressure_fragment.grib2)
  - matching manifest kept in [tests/fixtures/hrrr_demo_pressure_fragment.idx](../tests/fixtures/hrrr_demo_pressure_fragment.idx)
- Fixed HRRR demo column regression fixture:
  - [tests/fixtures/hrrr_demo_model_column.json](../tests/fixtures/hrrr_demo_model_column.json)
  - extracted from the fixed demo point `(1798, 1058)` after decoding the checked-in surface and pressure fragments
  - this is a frozen regression anchor generated by the current `rustbox` path, not an external truth dataset
- HRRR mesoanalysis fixture path:
  - reuses [tests/fixtures/hrrr_demo_pressure_fragment.grib2](../tests/fixtures/hrrr_demo_pressure_fragment.grib2)
  - reuses [tests/fixtures/hrrr_demo_pressure_fragment.idx](../tests/fixtures/hrrr_demo_pressure_fragment.idx)
  - current app demo decodes `HGT`, `TMP`, `UGRD`, and `VGRD` at `850 mb` to derive vorticity and pressure-level theta frontogenesis
- Legacy single-field render fixture:
  - [tests/fixtures/hrrr_gust_surface_fragment.grib2](../tests/fixtures/hrrr_gust_surface_fragment.grib2)
  - [tests/fixtures/hrrr_gust_surface_fragment.idx](../tests/fixtures/hrrr_gust_surface_fragment.idx)
- Sounding fixture source:
  - profile levels copied from `upstream/sharprs/tests/verification.rs` sounding 1
  - expected parcel values pinned to the current `rustbox` parcel implementation against `sharprs` commit `16cf0757304eb690d0208c304e32a4676178f00a`
  - expected severe kinematics pinned to the upstream `verification.rs` sounding 1 constants from the same commit
  - expected STP pinned to the current `rustbox` parcel + exact-layer local kinematic path
  - this is the closest thing to an external severe-truth anchor in the current repo
