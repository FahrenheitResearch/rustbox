# rustbox

`rustbox` is the local umbrella workspace for consolidating the weather Rust stack into one shared type system and one migration target.

This workspace is intentionally separate from the upstream repos cloned into `upstream/`. Those repos are the source material for migration work, not the long-term package boundary.

## Current direction

- `rustbox` is the umbrella workspace and local integration point.
- `metrust-py` remains an external compatibility surface on the Python side.
- The first implemented vertical slice is HRRR subset planning -> GRIB decode -> HRRR model-column extraction -> parcel and severe diagnostics -> transparent overlay render -> CLI demo.

## Workspace shape

```text
rustbox/
  crates/
    wx-types
    wx-geo
    wx-fetch
    wx-grib
    wx-thermo
    wx-grid
    wx-severe
    wx-radar
    wx-render
    wx-zarr
    wx-wrf
    wx-cuda
    wx-py
    wx-cli
  apps/
    mesoanalysis
    radar-viewer
    open-wx-api-rs
  python/
    metrust
    compat-cfgrib
    compat-eccodes
    compat-herbie
    compat-wrf
  docs/
  scripts/
  upstream/
```

## What Is Real Today

- `wx-fetch` can parse fixture-backed HRRR `.idx` manifests, distinguish source-object URLs from rebased fixture-fragment byte ranges, and error on ambiguous var/level selectors.
- `wx-grib` can decode real GRIB2 fixture fragments, including multi-message surface and pressure-level paths, and now fails if a selected slice contains anything other than exactly one GRIB2 message.
- `wx-grib` can extract one fixed HRRR model column into `wx-types::SoundingProfile`, with bundle-consistency checks across the decoded fields.
- `wx-thermo` computes real `sharprs`-derived SBCAPE, MLCAPE, MUCAPE, and CIN.
- `wx-grid` computes real constant-spacing finite-difference divergence, vorticity, advection, pressure-level theta frontogenesis, and MetPy-style 5/9-point smoothing from `Field2D` inputs.
- `wx-severe` computes fixed-layer STP plus exact-layer kinematics from a local compatibility fork of the pinned `sharprs` `winds.rs` logic, including a narrow endpoint-wind fallback where the direct upstream helicity path still fails on the checked-in fixture profile.
- `wx-render` writes a real transparent PNG overlay from a decoded scalar field.
- `cargo run -p wx-cli -- demo` now uses one coherent HRRR-based path from checked-in fixtures.
- `cargo run -p mesoanalysis-app -- demo` now decodes the checked-in HRRR 850 mb pressure fixture, computes vorticity plus theta-based frontogenesis on that pressure surface, and writes real PNG overlays for both.

## What Is Still Stubbed

- `wx-cuda` remains a capability stub only.
- `wx-radar`, `wx-wrf`, `wx-zarr`, and `wx-py` are not part of the implemented science surface yet.
- `radar-viewer` and `open-wx-api-rs` are still scaffold apps.
- Python compatibility packaging is not implemented.
- `rustbox` still depends on pinned upstream `grib-core` and `sharprs` crates rather than fully owning those cores locally.
- The direct pinned `sharprs::winds::helicity` call path still fails on the checked-in fixture profile; `rustbox` keeps a documented exact-layer local port while that upstream issue remains unresolved.
- `wx-grid` is still a constant-spacing finite-difference core. Projection-aware and map-factor-aware meteorological derivatives are future work.

## Truth Anchors

- `tests/fixtures/sounding_supercell.json` is the closest thing to an external science anchor in the current repo. Its severe kinematics are pinned to the upstream `sharprs` verification constants, while its STP is still pinned to the current `rustbox` parcel path plus the local severe compatibility fork.
- `tests/fixtures/hrrr_demo_model_column.json` is a frozen regression anchor, not an external truth source. It exists to make the fixed HRRR demo column reproducible and to catch accidental drift in extraction, parcel, and severe calculations for the implemented path.

## Notes

- `upstream/` is ignored by git and is meant for local clones of the source repos.
- `python/` is where compatibility packaging work will live as the Rust core stabilizes.
- `cargo run -p wx-cli -- status` reports what is real in the current slice and what remains stubbed.
