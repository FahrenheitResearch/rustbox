# Migration Notes

This file tracks the first real vertical slice that replaced the original scaffold.

## wx-types

- Upstream repo: local canonical core surface
- Commit: `rustbox`
- Source files:
  - local design work inside `rustbox`
- Adapted into:
  - [crates/wx-types/src/lib.rs](../crates/wx-types/src/lib.rs)
- What was adapted:
  - archive-facing container types for `Field3D`, `FieldBundle`, `NativeVolume`, `LevelAxis`, and `TimeAxis`
  - batch/archive job and manifest records for cycle planning, staging, and decode summaries
  - summary types for decoded 2D/3D field bundles
- What was intentionally left out:
  - projection-aware coordinate arrays
  - durable array-store metadata conventions beyond the current bundle/manifests

## wx-fetch

- Upstream repo: `rusbie`
- Commit: `bd13520b906993d58b1b29d27aaa8aae877a9338`
- Source files:
  - `upstream/rusbie/crates/herbie-core/src/idx.rs`
  - `upstream/rusbie/crates/herbie-core/src/cache.rs`
  - `upstream/rusbie/crates/herbie-core/src/client.rs`
  - `upstream/rusbie/crates/herbie-core/src/sources.rs`
- Adapted into:
  - [crates/wx-fetch/src/lib.rs](../crates/wx-fetch/src/lib.rs)
  - [crates/wx-fetch/src/cache.rs](../crates/wx-fetch/src/cache.rs)
  - [crates/wx-fetch/src/client.rs](../crates/wx-fetch/src/client.rs)
  - [crates/wx-fetch/src/sources.rs](../crates/wx-fetch/src/sources.rs)
- What was adapted:
  - `.idx` line parsing shape
  - HRRR multi-source URL construction pattern
  - variable/level subset selection semantics
  - multi-message request planning for rebased offline fixtures
  - explicit source-object vs rebased-fixture byte-range provenance
  - forecast-text normalization into forecast-hour semantics at planning time for analysis and simple hour/range forecasts
  - last-message range bounding through known fixture length
  - ambiguity rejection when a var/level selector matches multiple forecast entries
  - disk-cache keying and local cache layout
  - blocking HTTP client with retry, HEAD probing, and range-download support
  - cycle-range archive request expansion and rebased staged-subset manifests
- What was intentionally left out:
  - regex-heavy search surface
  - non-HRRR source catalogs
  - async orchestration beyond the current blocking archive client

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
  - staged-subset decode entry points for archive downloads
  - bundle summaries and 2D-to-3D field stacking for repeated pressure-level variables
  - HRRR-specific surface/pressure column extraction into `SoundingProfile`
  - bundle-consistency checks across decoded surface/pressure fields
  - a fixed regression-tested HRRR demo column at grid point `(1798, 1058)`
- What was intentionally left out:
  - full parser vendoring
  - JPEG2000/PNG/CCSDS feature plumbing in `rustbox`
  - generalized profile extraction beyond the current HRRR surface + pressure fixture path
  - native-model-specific derived metadata beyond stacked level bundles

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

- Upstream repo: `wrf-rust-plots` and `sharprs`
- Commit:
  - `wrf-rust-plots`: `2088e9599dcf7c7e55be5261c935a48c7afbdd60`
  - `sharprs`: `16cf0757304eb690d0208c304e32a4676178f00a`
- Source files:
  - `upstream/wrf-rust-plots/crates/wrf-render/src/colormaps.rs`
  - `upstream/wrf-rust-plots/crates/wrf-render/src/features.rs`
  - `upstream/wrf-rust-plots/crates/wrf-render/src/projection.rs`
  - `upstream/sharprs/src/render/compositor.rs`
  - `upstream/sharprs/src/render/skewt.rs`
  - `upstream/sharprs/src/render/hodograph.rs`
  - `upstream/sharprs/src/render/panels.rs`
  - `upstream/sharprs/src/render/param_table.rs`
- Adapted into:
  - [crates/wx-render/src/lib.rs](../crates/wx-render/src/lib.rs)
  - [crates/wx-render/src/map_render.rs](../crates/wx-render/src/map_render.rs)
  - [crates/wx-render/src/text.rs](../crates/wx-render/src/text.rs)
- What was adapted:
  - wind palette anchor colors
  - frontogenesis/vorticity diagnostic palettes
  - simple native raster overlay approach
  - projected basemap-backed map rendering over local `GridSpec` projection metadata
  - Natural Earth coastline, country-boundary, and state-line feature loading from checked-in assets
  - SHARPrs full-sounding PNG generation over local `SoundingProfile` conversion
- What was intentionally left out:
  - contour and wind-barb overlays on map products
  - richer map labeling and legends beyond the current title/colorbar surface
  - radar-to-basemap compositing

## wx-zarr

- Upstream repo: `open-mrms`
- Commit: `3d08c318e38b09f50734eea3434c08993dbaf499`
- Source files:
  - `upstream/open-mrms/crates/open-mrms-zarr/src/store.rs`
  - `upstream/open-mrms/crates/open-mrms-zarr/src/writer.rs`
- Adapted into:
  - [crates/wx-zarr/src/lib.rs](../crates/wx-zarr/src/lib.rs)
- What was adapted:
  - local Zarr v2 directory-store writer
  - `.zgroup`, `.zarray`, and `.zattrs` metadata layout
  - chunked compressed float chunk writing
  - per-store root attrs plus coordinate-array and field-array conventions
  - per-cycle archive-store path generation for decoded bundle outputs
  - local bundle reader that reconstructs `FieldBundle` values from persisted store metadata and chunks
- What was intentionally left out:
  - multi-cycle append into a single logical time axis
  - richer CF/grid-mapping conventions for projected model grids
  - remote/object-store backends

## wx-radar

- Upstream repo: `rustdar`
- Commit: `3a708877b3f7bb5a93e0d14a95df36a3e0ada698`
- Source files:
  - `upstream/rustdar/crates/rustdar-core/src/level2.rs`
  - `upstream/rustdar/crates/rustdar-core/src/products.rs`
  - `upstream/rustdar/crates/rustdar-core/src/sites.rs`
  - `upstream/rustdar/crates/rustdar-core/src/color_table.rs`
  - `upstream/rustdar/crates/rustdar-core/src/render.rs`
  - `upstream/rustdar/src/nexrad/derived.rs`
  - `upstream/rustdar/src/nexrad/srv.rs`
  - `upstream/rustdar/src/nexrad/detection.rs`
- Adapted into:
  - [crates/wx-radar/src/lib.rs](../crates/wx-radar/src/lib.rs)
  - [crates/wx-radar/src/nexrad/mod.rs](../crates/wx-radar/src/nexrad/mod.rs)
  - [crates/wx-radar/src/nexrad/level2.rs](../crates/wx-radar/src/nexrad/level2.rs)
  - [crates/wx-radar/src/nexrad/products.rs](../crates/wx-radar/src/nexrad/products.rs)
  - [crates/wx-radar/src/nexrad/sites.rs](../crates/wx-radar/src/nexrad/sites.rs)
  - [crates/wx-radar/src/nexrad/color_table.rs](../crates/wx-radar/src/nexrad/color_table.rs)
  - [crates/wx-radar/src/nexrad/render.rs](../crates/wx-radar/src/nexrad/render.rs)
  - [crates/wx-radar/src/nexrad/derived.rs](../crates/wx-radar/src/nexrad/derived.rs)
  - [crates/wx-radar/src/nexrad/srv.rs](../crates/wx-radar/src/nexrad/srv.rs)
  - [crates/wx-radar/src/nexrad/detection.rs](../crates/wx-radar/src/nexrad/detection.rs)
- What was adapted:
  - Level II archive-volume parsing and product inventory
  - radar-site catalog and nearest-site helpers
  - palette tables plus classic/smooth radar sweep rendering
  - derived VIL, echo tops, and storm-relative velocity
  - rotation, TVS, and hail detection summaries
  - a thin public API over the imported radar core for file reads, summaries, rendering, and detection
- What was intentionally left out:
  - live radar download/orchestration inside `wx-radar`
  - interactive WGPU viewer integration
  - storm tracking/alerting assimilation beyond the current detection summary surface

## mesoanalysis app

- Upstream repo: `metrust-py`
- Commit: `1b8436779832be326f24bd2acd40a908e9f54685`
- Source files:
  - `upstream/metrust-py/docs/methodology/architecture.md`
  - `upstream/metrust-py/docs/methodology/grid-kinematics.md`
  - `upstream/metrust-py/crates/metrust/src/calc/kinematics.rs`
- Adapted into:
  - [apps/mesoanalysis/src/main.rs](../apps/mesoanalysis/src/main.rs)
- What was adapted:
  - batch-oriented product registry shape over a local archive/persisted-store core
  - per-cycle product dependency mapping for 850 mb wind/temperature diagnostics
  - archive-manifest input -> persisted-store read -> derived-store write -> PNG export flow
  - local run/resume manifest for mesoanalysis product batches
- What was intentionally left out:
  - the full upstream hrrr-mesoanalysis 46-parameter orchestration
  - obs/QC/Barnes assimilation
  - projection-aware render products beyond the current PNG overlays

## radar-viewer app

- Upstream repo: `rustdar`
- Commit: `3a708877b3f7bb5a93e0d14a95df36a3e0ada698`
- Source files:
  - `upstream/rustdar/src/bin/radar_render.rs`
- Adapted into:
  - [apps/radar-viewer/src/main.rs](../apps/radar-viewer/src/main.rs)
- What was adapted:
  - thin file-oriented inspect/render/detect CLI shape over the local radar core
  - product parsing, render-mode selection, and PNG export flow
- What was intentionally left out:
  - live NEXRAD S3 download flow
  - map overlays and richer UI surface

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
- Archive smoke-test path:
  - `cargo run -p wx-cli -- archive-run 2024040100 2024040100 prs 0 target/archive-smoke "TMP|850 mb|anl" "TMP|700 mb|anl"`
  - fetches a real remote HRRR pressure subset, stages a rebased local GRIB fragment, writes a decoded bundle summary manifest under `target/archive-smoke/decoded/`, and persists a Zarr store under `target/archive-smoke/zarr/`
- HRRR mesoanalysis fixture path:
  - reuses [tests/fixtures/hrrr_demo_pressure_fragment.grib2](../tests/fixtures/hrrr_demo_pressure_fragment.grib2)
  - reuses [tests/fixtures/hrrr_demo_pressure_fragment.idx](../tests/fixtures/hrrr_demo_pressure_fragment.idx)
  - current app demo decodes `HGT`, `TMP`, `UGRD`, and `VGRD` at `850 mb` to derive smoothed vorticity, divergence, temperature advection, and pressure-level theta frontogenesis
- Archive-backed mesoanalysis smoke-test path:
  - `cargo run -p wx-cli -- archive-run 2024040100 2024040100 prs 0 target/meso-batch-smoke "HGT|850 mb|anl" "TMP|850 mb|anl" "UGRD|850 mb|anl" "VGRD|850 mb|anl"`
  - `cargo run -p mesoanalysis-app -- run target/meso-batch-smoke/archive_manifest.json target/meso-products-smoke all`
  - stages a real remote HRRR subset, persists the base per-cycle store, then writes a derived per-cycle mesoanalysis Zarr store plus PNG outputs
- Basemap assets:
  - checked-in Natural Earth 110m layers under [assets/basemap/natural_earth_110m](../assets/basemap/natural_earth_110m)
  - sourced from Natural Earth public-domain downloads for coastline, country-boundary, and state/province linework
- Radar fixture source:
  - `https://unidata-nexrad-level2.s3.amazonaws.com/2024/01/01/KATX/KATX20240101_000258_V06`
  - trimmed into [tests/fixtures/KATX20240101_000258_partial_V06](../tests/fixtures/KATX20240101_000258_partial_V06) to keep offline radar tests fast while preserving a real Level II volume
- Radar smoke-test path:
  - `cargo run -p radar-viewer-app -- inspect tests/fixtures/KATX20240101_000258_partial_V06`
  - `cargo run -p radar-viewer-app -- render tests/fixtures/KATX20240101_000258_partial_V06 REF target/demo/radar_reflectivity.png 0 512 classic default`
  - `cargo run -p radar-viewer-app -- detect tests/fixtures/KATX20240101_000258_partial_V06`
- Legacy single-field render fixture:
  - [tests/fixtures/hrrr_gust_surface_fragment.grib2](../tests/fixtures/hrrr_gust_surface_fragment.grib2)
  - [tests/fixtures/hrrr_gust_surface_fragment.idx](../tests/fixtures/hrrr_gust_surface_fragment.idx)
- Sounding fixture source:
  - profile levels copied from `upstream/sharprs/tests/verification.rs` sounding 1
  - expected parcel values pinned to the current `rustbox` parcel implementation against `sharprs` commit `16cf0757304eb690d0208c304e32a4676178f00a`
  - expected severe kinematics pinned to the upstream `verification.rs` sounding 1 constants from the same commit
  - expected STP pinned to the current `rustbox` parcel + exact-layer local kinematic path
  - this is the closest thing to an external severe-truth anchor in the current repo
