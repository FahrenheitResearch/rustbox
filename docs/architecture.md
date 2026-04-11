# rustbox Architecture

## Goal

`rustbox` is the workspace root for converging the existing weather repos into a single Rust-first platform with shared types and thin compatibility surfaces.

## Core rules

1. CPU Rust is the source of truth.
2. GPU acceleration is optional and only lands in targeted hotspots.
3. Shared types come first; wrappers come second.
4. Upstream repos stay isolated until code is intentionally migrated.

## Crate responsibilities

- `wx-types`: canonical fields, profiles, observations, levels, and metadata.
- `wx-geo`: geodesics and coordinate helpers.
- `wx-fetch`: source-aware download planning and cache-facing fetch orchestration.
- `wx-grib`: GRIB subset description, decode entry points, and field extraction.
- `wx-thermo`: parcel math, CAPE/CIN, ECAPE-oriented hooks, and sounding diagnostics.
- `wx-grid`: derived grid math such as vorticity, divergence, smoothing, and frontogenesis.
- `wx-severe`: composite severe-weather indices built from thermo and kinematic primitives.
- `wx-radar`: radar volumes, gates, and storm-analysis-facing abstractions.
- `wx-render`: rendered overlays, sounding products, and map-ready outputs.
- `wx-zarr`: Zarr export metadata and write-path hooks.
- `wx-wrf`: WRF-specific adapters and shape conversions.
- `wx-cuda`: optional acceleration dispatch and capability reporting.
- `wx-py`: shared Python compatibility surface over the Rust core.
- `wx-cli`: integration binary for vertical-slice execution and smoke tests.

## Immediate vertical slice

The workspace skeleton is aimed at this first end-to-end path:

1. Build an HRRR subset request.
2. Decode a GRIB subset into a canonical field grid.
3. Compute parcel diagnostics and a severe composite.
4. Render a projected map product and a sounding product.
5. Expose the same path through CLI and Python compatibility layers.
