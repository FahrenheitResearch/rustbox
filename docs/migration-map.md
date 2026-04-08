# Migration Map

This map is a starting point for intentional migration into `rustbox`.

| Upstream repo | Primary landing area in rustbox | Reason |
| --- | --- | --- |
| `metrust-py` | `wx-grid`, `wx-py`, `python/metrust` | Broad calc API, units-facing Python compatibility, and packaging surface |
| `sharprs` | `wx-thermo`, `wx-severe`, `wx-render` | Sounding math, parcel logic, severe diagnostics, sounding rendering |
| `ecape-rs` | `wx-thermo` | Entraining parcel and ECAPE path |
| `cfrust` | `wx-grib` | GRIB decode core and Python-facing GRIB lessons |
| `ecrust` | `wx-grib`, `python/compat-eccodes` | ecCodes compatibility layer and repack/write ideas |
| `rusbie` | `wx-fetch`, `python/compat-herbie` | HRRR subset fetch patterns and `.idx`-driven planning |
| `rustdar` | `wx-radar`, `apps/radar-viewer` | Radar parsing, WGPU rendering patterns, viewer UX |
| `wrf-rust` | `wx-wrf` | WRF-specific adapters and diagnostics |
| `wrf-rust-plots` | `wx-render` | Raster map rendering and weather-plot composition |
| `geors` | `wx-geo` | Geodesic helpers and WGS84 primitives |
| `geocat-rs` | `wx-geo`, `wx-thermo` | Select interpolation and humidity/dewpoint helpers |
| `met-cu` | `wx-cuda` | Candidate GPU kernels and parity tests |
| `open-mrms` | `wx-radar`, `wx-zarr` | Storm tracking and downstream export patterns |

## Compatibility note

The public Python package name remains tied to `metrust-py` today. `rustbox` should treat that as a compatibility surface, not as the internal workspace identity.

