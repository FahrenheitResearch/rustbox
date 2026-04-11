# rustbox

`rustbox` is the local umbrella workspace for consolidating the weather Rust stack into one shared type system and one migration target.

This workspace is intentionally separate from the upstream repos cloned into `upstream/`. Those repos are the source material for migration work, not the long-term package boundary.

For a quick visual tour of the current implemented surface, open [docs/showcase.md](docs/showcase.md).

## Current direction

- `rustbox` is the umbrella workspace and local integration point.
- `metrust-py` remains an external compatibility surface on the Python side.
- The first implemented vertical slice is HRRR subset planning -> GRIB decode -> HRRR model-column extraction -> parcel and severe diagnostics -> basemap map render plus SHARPrs sounding render -> CLI demo.
- The archive-core focus is now HRRR planning -> remote subset staging -> bundle decode -> per-cycle Zarr persistence -> batch product generation, still within the same crate layout.

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

- `wx-types` now carries real archive-facing bundle/container types: `Field3D`, `FieldBundle`, `NativeVolume`, `TimeAxis`, `LevelAxis`, and batch/archive manifest types.
- `wx-fetch` can now probe real HRRR sources, fetch remote `.idx` manifests, plan byte-range subsets, cache downloads, iterate cycle ranges, and stage rebased subset files for later decode.
- `wx-fetch` can parse fixture-backed HRRR `.idx` manifests, distinguish source-object URLs from rebased fixture-fragment byte ranges, and error on ambiguous var/level selectors.
- `wx-grib` can decode real GRIB2 fixture fragments, including multi-message surface and pressure-level paths, and now fails if a selected slice contains anything other than exactly one GRIB2 message.
- `wx-grib` can decode staged HRRR subset files into real `FieldBundle` summaries and stack repeated pressure levels into `Field3D` volumes.
- `wx-grib` can extract one fixed HRRR model column into `wx-types::SoundingProfile`, with bundle-consistency checks across the decoded fields and an optional lake-interpolation seam for 2 m thermo when a decoded land/sea mask is available.
- `wx-thermo` computes real `sharprs`-derived SBCAPE, MLCAPE, MUCAPE, and CIN.
- `wx-grid` computes real constant-spacing finite-difference divergence, vorticity, advection, pressure-level theta frontogenesis, and MetPy-style 5/9-point smoothing from `Field2D` inputs.
- `wx-severe` computes fixed-layer STP plus exact-layer kinematics from a local compatibility fork of the pinned `sharprs` `winds.rs` logic, including a narrow endpoint-wind fallback where the direct upstream helicity path still fails on the checked-in fixture profile.
- `wx-render` writes real PNG products from decoded scalar fields, including raw transparent overlays, projected basemap-backed map renders, SHARPrs full-sounding analysis PNGs, anti-aliased TTF text rendering, and product-aware leveled colormaps adapted from `wrf-rust-plots`.
- `wx-zarr` now writes and reads real per-cycle Zarr v2 directory stores for decoded `FieldBundle` outputs, including 2D/3D arrays, coordinate arrays, root attrs, archive-store descriptors, and bundle round-tripping.
- `wx-radar` now provides a real rustdar-backed NEXRAD Level II surface: volume parsing, product inventory, derived products, detection summaries, and radar rendering.
- `cargo run -p wx-cli -- demo` now uses one coherent HRRR-based path from checked-in fixtures and emits a raw gust overlay, a basemap-backed gust map, and a SHARPrs full-sounding PNG from the extracted model column.
- `cargo run -p wx-cli -- plan ...`, `download ...`, `decode ...`, `archive-run ...`, and `resume ...` now provide a real archive-facing HRRR core for planning, staging, decoding, and persisting remote subset jobs.
- `cargo run -p mesoanalysis-app -- demo` now decodes the checked-in HRRR 850 mb pressure fixture, computes a real 850 mb product bundle, writes derived Zarr output, and renders four basemap-backed diagnostic PNGs.
- `cargo run -p mesoanalysis-app -- run <archive_manifest.json> <output_root> [product...]` now consumes persisted archive-cycle Zarr stores and generates real per-cycle mesoanalysis Zarr + PNG outputs for 850 mb smoothed vorticity, divergence, temperature advection, and theta frontogenesis.
- `cargo run -p radar-viewer-app -- inspect <level2_file>`, `render ...`, and `detect ...` now provide a thin real CLI over the local radar core for offline Level II inspection, PNG export, and signature summaries.

## What Is Still Stubbed

- `wx-cuda` remains a capability stub only.
- `wx-wrf` and `wx-py` are not part of the implemented science surface yet.
- `open-wx-api-rs` is still a scaffold app.
- Python compatibility packaging is not implemented.
- The archive core now persists per-cycle Zarr stores, but multi-cycle append/partition semantics and richer archive-wide store conventions are still early.
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
