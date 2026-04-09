use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Rgba, RgbaImage};
use std::path::{Path, PathBuf};
use wx_types::Field2D;

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

#[derive(Debug, Clone, PartialEq)]
pub struct OverlaySpec {
    pub palette: String,
    pub transparent_background: bool,
    pub value_range: Option<(f32, f32)>,
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

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    image
        .save(output_path)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    Ok(RenderedOverlay {
        output_path: output_path.to_path_buf(),
        width: image.width(),
        height: image.height(),
        palette: spec.palette.clone(),
        value_min,
        value_max,
    })
}

fn palette_by_name(name: &str) -> Result<Vec<Rgba<u8>>> {
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
        other => bail!("unsupported palette '{}'", other),
    }
}

fn color_for_value(palette: &[Rgba<u8>], normalized: f32) -> Rgba<u8> {
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
    use wx_grib::decode_selected_message;

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
}
