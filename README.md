# rustbox

`rustbox` is the local umbrella workspace for consolidating the weather Rust stack into one shared type system and one migration target.

This workspace is intentionally separate from the upstream repos cloned into `upstream/`. Those repos are the source material for migration work, not the long-term package boundary.

## Current direction

- `rustbox` is the umbrella workspace and local integration point.
- `metrust-py` remains an external compatibility surface on the Python side.
- The first vertical slice is HRRR subset fetch -> GRIB decode -> parcel and severe diagnostics -> overlay render -> CLI/Python exposure.

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

## Notes

- `upstream/` is ignored by git and is meant for local clones of the source repos.
- `python/` is where compatibility packaging work will live as the Rust core stabilizes.
- `cargo run -p wx-cli -- status` gives a quick sanity check that the workspace is wired.

