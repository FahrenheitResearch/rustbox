mod map_render;
mod text;

use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Rgba, RgbaImage};
use sharprs::render::{compute_all_params, render_full_sounding};
use std::path::{Path, PathBuf};
use wx_thermo::to_sharprs_profile;
use wx_types::{Field2D, SoundingProfile};

pub use map_render::render_field_to_map_png;

const WINDS_PALETTE: &[&str] = &[
    "#ffffff", "#87cefa", "#6a5acd", "#e696dc", "#c85abe", "#a01496", "#c80028", "#dc283c",
    "#f05050", "#faf064", "#dcbe46", "#be8c28", "#a05a0a",
];
const VORTICITY_PALETTE: &[&str] = &[
    "#1f3b73", "#2f5d9b", "#4d88bf", "#84b6d8", "#d7e8f4", "#f8f8f8", "#f4d3cf", "#dd8f84",
    "#c8574c", "#a62b26", "#7f0000",
];
const FRONTOGENESIS_PALETTE: &[&str] = &[
    "#183a63", "#2e5f92", "#5f90bf", "#a9c9e6", "#f7f7f7", "#f0c7b8", "#da8f78", "#c45c48",
    "#9e2f2f", "#6f0f1f",
];
const DIVERGENCE_PALETTE: &[&str] = &[
    "#113d6b", "#2d6ea0", "#6aa4c8", "#d7ebf5", "#f7f7f7", "#f3d4c7", "#d98b6f", "#b54837",
    "#7d1822",
];
const ADVECTION_PALETTE: &[&str] = &[
    "#0b3c5d", "#328cc1", "#74b3ce", "#d9ecf2", "#f7f7f7", "#f3d9ca", "#e39b7b", "#c75d43",
    "#8f2d1f",
];

#[derive(Debug, Clone, PartialEq)]
pub struct OverlaySpec {
    pub palette: String,
    pub transparent_background: bool,
    pub value_range: Option<(f32, f32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapMarker {
    pub grid_x: usize,
    pub grid_y: usize,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapOverlaySpec {
    pub palette: String,
    pub value_range: Option<(f32, f32)>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub colorbar_label: Option<String>,
    pub markers: Vec<MapMarker>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundingRenderSpec {
    pub output_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderedImage {
    pub output_path: PathBuf,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderedOverlay {
    pub output_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub palette: String,
    pub value_min: f32,
    pub value_max: f32,
}

pub fn render_field_to_png(
    field: &Field2D,
    spec: &OverlaySpec,
    output_path: &Path,
) -> Result<RenderedOverlay> {
    let expected_len = field.expected_len();
    if field.values.len() != expected_len {
        bail!(
            "field value count {} does not match grid {}x{}",
            field.values.len(),
            field.grid.nx,
            field.grid.ny
        );
    }

    let (value_min, value_max) = spec
        .value_range
        .or_else(|| field.finite_min_max())
        .context("field did not contain any finite values to render")?;
    let value_span = if (value_max - value_min).abs() < f32::EPSILON {
        1.0
    } else {
        value_max - value_min
    };
    let palette = palette_by_name(&spec.palette)?;

    let mut image: RgbaImage = ImageBuffer::new(field.grid.nx as u32, field.grid.ny as u32);
    for (index, pixel) in image.pixels_mut().enumerate() {
        let value = field.values[index];
        *pixel = if !value.is_finite() {
            Rgba([0, 0, 0, 0])
        } else {
            let normalized = ((value - value_min) / value_span).clamp(0.0, 1.0);
            let mut color = color_for_value(&palette, normalized);
            color.0[3] = if spec.transparent_background {
                (normalized * 220.0).round() as u8
            } else {
                u8::MAX
            };
            color
        };
    }

    write_image(&image, output_path)?;

    Ok(RenderedOverlay {
        output_path: output_path.to_path_buf(),
        width: image.width(),
        height: image.height(),
        palette: spec.palette.clone(),
        value_min,
        value_max,
    })
}

pub fn render_sounding_to_png(
    profile: &SoundingProfile,
    spec: &SoundingRenderSpec,
) -> Result<RenderedImage> {
    let sharp_profile = to_sharprs_profile(profile)?;
    let params = compute_all_params(&sharp_profile);
    let png_bytes = render_full_sounding(&sharp_profile, &params);

    if let Some(parent) = spec.output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(&spec.output_path, &png_bytes)
        .with_context(|| format!("failed to write {}", spec.output_path.display()))?;

    let image = image::load_from_memory(&png_bytes).context("failed to decode sounding PNG")?;
    Ok(RenderedImage {
        output_path: spec.output_path.clone(),
        width: image.width(),
        height: image.height(),
    })
}

fn write_image(image: &RgbaImage, output_path: &Path) -> Result<()> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    image
        .save(output_path)
        .with_context(|| format!("failed to write {}", output_path.display()))?;
    Ok(())
}

pub(crate) fn palette_by_name(name: &str) -> Result<Vec<Rgba<u8>>> {
    match name {
        "winds" => Ok(WINDS_PALETTE.iter().map(|hex| rgba_from_hex(hex)).collect()),
        "vorticity" => Ok(VORTICITY_PALETTE
            .iter()
            .map(|hex| rgba_from_hex(hex))
            .collect()),
        "frontogenesis" => Ok(FRONTOGENESIS_PALETTE
            .iter()
            .map(|hex| rgba_from_hex(hex))
            .collect()),
        "divergence" => Ok(DIVERGENCE_PALETTE
            .iter()
            .map(|hex| rgba_from_hex(hex))
            .collect()),
        "advection" => Ok(ADVECTION_PALETTE
            .iter()
            .map(|hex| rgba_from_hex(hex))
            .collect()),
        other => bail!("unsupported palette '{}'", other),
    }
}

pub(crate) fn color_for_value(palette: &[Rgba<u8>], normalized: f32) -> Rgba<u8> {
    let max_index = palette.len().saturating_sub(1);
    let index = ((max_index as f32) * normalized).round() as usize;
    palette[index.min(max_index)]
}

fn rgba_from_hex(value: &str) -> Rgba<u8> {
    let trimmed = value.trim_start_matches('#');
    let red = u8::from_str_radix(&trimmed[0..2], 16).expect("valid red component");
    let green = u8::from_str_radix(&trimmed[2..4], 16).expect("valid green component");
    let blue = u8::from_str_radix(&trimmed[4..6], 16).expect("valid blue component");
    Rgba([red, green, blue, u8::MAX])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;
    use tempfile::TempDir;
    use wx_fetch::{HrrrSelectionRequest, HrrrSubsetRequest, plan_hrrr_subset};
    use wx_grib::{build_hrrr_sounding_profile, decode_selected_message, decode_selected_messages};

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[test]
    fn renderer_writes_non_empty_png() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_gust_surface_fragment.idx"))
            .expect("fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let plan = plan_hrrr_subset(
            &HrrrSubsetRequest {
                cycle,
                forecast_hour: 0,
                product: "sfc".to_string(),
                selections: vec![HrrrSelectionRequest {
                    variable: "GUST".to_string(),
                    level: "surface".to_string(),
                    forecast: None,
                }],
            },
            &idx_text,
        )
        .expect("plan should succeed");
        let field =
            decode_selected_message(&fixture_path("hrrr_gust_surface_fragment.grib2"), &plan)
                .expect("decode should succeed");

        let temp_dir = TempDir::new().expect("tempdir should work");
        let output_path = temp_dir.path().join("gust_overlay.png");
        let overlay = render_field_to_png(
            &field,
            &OverlaySpec {
                palette: "winds".to_string(),
                transparent_background: true,
                value_range: None,
            },
            &output_path,
        )
        .expect("render should succeed");

        let bytes = std::fs::read(&overlay.output_path).expect("png should be readable");
        assert!(bytes.len() > 100);
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn map_renderer_writes_non_empty_png() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_gust_surface_fragment.idx"))
            .expect("fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let plan = plan_hrrr_subset(
            &HrrrSubsetRequest {
                cycle,
                forecast_hour: 0,
                product: "sfc".to_string(),
                selections: vec![HrrrSelectionRequest {
                    variable: "GUST".to_string(),
                    level: "surface".to_string(),
                    forecast: None,
                }],
            },
            &idx_text,
        )
        .expect("plan should succeed");
        let field =
            decode_selected_message(&fixture_path("hrrr_gust_surface_fragment.grib2"), &plan)
                .expect("decode should succeed");

        let temp_dir = TempDir::new().expect("tempdir should work");
        let output_path = temp_dir.path().join("gust_map.png");
        let overlay = render_field_to_map_png(
            &field,
            &MapOverlaySpec {
                palette: "winds".to_string(),
                value_range: None,
                title: Some("Test Gust".to_string()),
                subtitle: Some("fixture map".to_string()),
                colorbar_label: Some("m/s".to_string()),
                markers: vec![MapMarker {
                    grid_x: 1_798,
                    grid_y: 1_058,
                    label: Some("profile".to_string()),
                }],
            },
            &output_path,
        )
        .expect("map render should succeed");

        let bytes = std::fs::read(&overlay.output_path).expect("png should be readable");
        assert!(bytes.len() > 1000);
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn sounding_renderer_writes_full_png() {
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let surface_plan = wx_fetch::plan_hrrr_fixture_subset(
            &HrrrSubsetRequest {
                cycle,
                forecast_hour: 0,
                product: "sfc".to_string(),
                selections: vec![
                    HrrrSelectionRequest {
                        variable: "PRES".to_string(),
                        level: "surface".to_string(),
                        forecast: Some("anl".to_string()),
                    },
                    HrrrSelectionRequest {
                        variable: "HGT".to_string(),
                        level: "surface".to_string(),
                        forecast: Some("anl".to_string()),
                    },
                    HrrrSelectionRequest {
                        variable: "TMP".to_string(),
                        level: "2 m above ground".to_string(),
                        forecast: Some("anl".to_string()),
                    },
                    HrrrSelectionRequest {
                        variable: "DPT".to_string(),
                        level: "2 m above ground".to_string(),
                        forecast: Some("anl".to_string()),
                    },
                    HrrrSelectionRequest {
                        variable: "UGRD".to_string(),
                        level: "10 m above ground".to_string(),
                        forecast: Some("anl".to_string()),
                    },
                    HrrrSelectionRequest {
                        variable: "VGRD".to_string(),
                        level: "10 m above ground".to_string(),
                        forecast: Some("anl".to_string()),
                    },
                ],
            },
            &std::fs::read_to_string(fixture_path("hrrr_demo_surface_fragment.idx"))
                .expect("surface idx should be readable"),
            std::fs::metadata(fixture_path("hrrr_demo_surface_fragment.grib2"))
                .expect("surface fragment should exist")
                .len(),
        )
        .expect("surface plan should succeed");
        let pressure_plan = wx_fetch::plan_hrrr_fixture_subset(
            &HrrrSubsetRequest {
                cycle,
                forecast_hour: 0,
                product: "prs".to_string(),
                selections: [
                    "1000 mb", "925 mb", "850 mb", "700 mb", "500 mb", "400 mb", "300 mb",
                ]
                .into_iter()
                .flat_map(|level| {
                    [
                        HrrrSelectionRequest {
                            variable: "HGT".to_string(),
                            level: level.to_string(),
                            forecast: Some("anl".to_string()),
                        },
                        HrrrSelectionRequest {
                            variable: "TMP".to_string(),
                            level: level.to_string(),
                            forecast: Some("anl".to_string()),
                        },
                        HrrrSelectionRequest {
                            variable: "DPT".to_string(),
                            level: level.to_string(),
                            forecast: Some("anl".to_string()),
                        },
                        HrrrSelectionRequest {
                            variable: "UGRD".to_string(),
                            level: level.to_string(),
                            forecast: Some("anl".to_string()),
                        },
                        HrrrSelectionRequest {
                            variable: "VGRD".to_string(),
                            level: level.to_string(),
                            forecast: Some("anl".to_string()),
                        },
                    ]
                })
                .collect(),
            },
            &std::fs::read_to_string(fixture_path("hrrr_demo_pressure_fragment.idx"))
                .expect("pressure idx should be readable"),
            std::fs::metadata(fixture_path("hrrr_demo_pressure_fragment.grib2"))
                .expect("pressure fragment should exist")
                .len(),
        )
        .expect("pressure plan should succeed");

        let surface_messages = decode_selected_messages(
            &fixture_path("hrrr_demo_surface_fragment.grib2"),
            &surface_plan,
        )
        .expect("surface decode should succeed");
        let pressure_messages = decode_selected_messages(
            &fixture_path("hrrr_demo_pressure_fragment.grib2"),
            &pressure_plan,
        )
        .expect("pressure decode should succeed");
        let profile =
            build_hrrr_sounding_profile(&surface_messages, &pressure_messages, 1_798, 1_058)
                .expect("profile extraction should succeed");

        let temp_dir = TempDir::new().expect("tempdir should work");
        let rendered = render_sounding_to_png(
            &profile,
            &SoundingRenderSpec {
                output_path: temp_dir.path().join("sounding.png"),
            },
        )
        .expect("sounding render should succeed");

        let bytes = std::fs::read(&rendered.output_path).expect("png should be readable");
        assert!(bytes.len() > 10_000);
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(rendered.width, 2400);
        assert_eq!(rendered.height, 1800);
    }
}
