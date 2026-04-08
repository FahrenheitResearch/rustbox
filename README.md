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

- `wx-fetch` can parse fixture-backed HRRR `.idx` manifests and plan single or multi-message subsets.
- `wx-grib` can decode real GRIB2 fixture fragments, including multi-message surface and pressure-level paths.
- `wx-grib` can extract one deterministic HRRR model column into `wx-types::SoundingProfile`.
- `wx-thermo` computes real `sharprs`-derived SBCAPE, MLCAPE, MUCAPE, and CIN.
- `wx-severe` computes fixed-layer STP plus supporting kinematics from a local `sharprs`-derived implementation.
- `wx-render` writes a real transparent PNG overlay from a decoded scalar field.
- `cargo run -p wx-cli -- demo` now uses one coherent HRRR-based path from checked-in fixtures.

## What Is Still Stubbed

- `wx-cuda` remains a capability stub only.
- `wx-radar`, `wx-wrf`, `wx-zarr`, and `wx-py` are not part of the implemented slice yet.
- Python compatibility packaging is not implemented.
- The canonical pinned `sharprs` `winds.rs` path is not yet the source of truth for severe kinematics in this repo; `rustbox` currently uses a documented local adaptation from `sharprs/src/python.rs`.

## Notes

- `upstream/` is ignored by git and is meant for local clones of the source repos.
- `python/` is where compatibility packaging work will live as the Rust core stabilizes.
- `cargo run -p wx-cli -- status` reports what is real in the current slice and what remains stubbed.
