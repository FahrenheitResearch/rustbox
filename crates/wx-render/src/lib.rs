mod colormap;
mod colormaps;
mod map_render;
mod style;
mod text;

use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Rgba, RgbaImage};
use sharprs::render::{compute_all_params, render_full_sounding};
use std::path::{Path, PathBuf};
use style::resolve_render_style;
use wx_thermo::to_sharprs_profile;
use wx_types::{Field2D, SoundingProfile};

pub use map_render::render_field_to_map_png;

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
    let style = resolve_render_style(&spec.palette, spec.value_range)?;

    let mut image: RgbaImage = ImageBuffer::new(field.grid.nx as u32, field.grid.ny as u32);
    for render_y in 0..field.grid.ny {
        for x in 0..field.grid.nx {
            let value = field.values[render_y * field.grid.nx + x];
            let pixel = image.get_pixel_mut(x as u32, render_y as u32);
            *pixel = if !value.is_finite() {
                Rgba([0, 0, 0, 0])
            } else {
                let mut color = style.color_for_value(value as f64);
                if color.0[3] == 0 {
                    Rgba([0, 0, 0, 0])
                } else {
                    let normalized = style.normalized_value(value as f64).unwrap_or(1.0);
                    color.0[3] = if spec.transparent_background {
                        (40.0 + normalized * 180.0).round().clamp(0.0, 220.0) as u8
                    } else {
                        u8::MAX
                    };
                    color
                }
            };
        }
    }

    write_image(&image, output_path)?;

    Ok(RenderedOverlay {
        output_path: output_path.to_path_buf(),
        width: image.width(),
        height: image.height(),
        palette: style.id,
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
