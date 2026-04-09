use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use std::path::PathBuf;
use wx_fetch::{HrrrSelectionRequest, HrrrSubsetRequest, plan_hrrr_fixture_subset};
use wx_grib::decode_selected_messages;
use wx_grid::{field_stats, frontogenesis_field, smooth_n_point_field, vorticity_field};
use wx_render::{OverlaySpec, render_field_to_png};
use wx_types::Field2D;

fn main() -> Result<()> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "status".to_string());

    match command.as_str() {
        "status" => print_status(),
        "demo" => run_demo()?,
        _ => {
            println!("usage: cargo run -p mesoanalysis-app -- [status|demo]");
        }
    }

    Ok(())
}

fn print_status() {
    println!("mesoanalysis: real offline HRRR pressure-level demo");
    println!("mesoanalysis: decodes 850 mb TMP/UGRD/VGRD from checked-in GRIB2 fixtures");
    println!("mesoanalysis: computes real wx-grid vorticity, smoothing, and frontogenesis");
    println!("mesoanalysis: writes a transparent vorticity PNG to target/demo");
}

fn run_demo() -> Result<()> {
    let cycle = Utc
        .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
        .single()
        .expect("valid fixture cycle");
    let fixture_root = repo_root().join("tests/fixtures");
    let fragment = fixture_root.join("hrrr_demo_pressure_fragment.grib2");
    let idx_text = std::fs::read_to_string(fixture_root.join("hrrr_demo_pressure_fragment.idx"))
        .context("failed to read pressure fixture idx")?;
    let plan = plan_hrrr_fixture_subset(
        &pressure_demo_request(cycle),
        &idx_text,
        std::fs::metadata(&fragment)
            .context("failed to stat pressure fixture")?
            .len(),
    )?;

    let decoded = decode_selected_messages(&fragment, &plan)?;
    let temperature = select_field(&decoded, "TMP", "850 mb")?;
    let u_wind = select_field(&decoded, "UGRD", "850 mb")?;
    let v_wind = select_field(&decoded, "VGRD", "850 mb")?;

    let vorticity = vorticity_field(u_wind, v_wind)?;
    let smoothed_vorticity = smooth_n_point_field(&vorticity, 9, 1)?;
    let frontogenesis = frontogenesis_field(temperature, u_wind, v_wind)?;

    let output_path = repo_root().join("target/demo/mesoanalysis_850mb_vorticity.png");
    let overlay = render_field_to_png(
        &smoothed_vorticity,
        &OverlaySpec {
            palette: "vorticity".to_string(),
            transparent_background: true,
            value_range: None,
        },
        &output_path,
    )?;

    let vort_stats = field_stats(&vorticity).context("vorticity contained no finite values")?;
    let smooth_stats = field_stats(&smoothed_vorticity)
        .context("smoothed vorticity contained no finite values")?;
    let fronto_stats =
        field_stats(&frontogenesis).context("frontogenesis contained no finite values")?;

    println!(
        "source_range_origin={} pressure_source_grib_url={}",
        plan.byte_range_origin.label(),
        plan.source_grib_url
    );
    println!(
        "level={} grid={}x{} vort_range={:.6}..{:.6}",
        smoothed_vorticity.metadata.level.description,
        smoothed_vorticity.grid.nx,
        smoothed_vorticity.grid.ny,
        smooth_stats.min_value,
        smooth_stats.max_value
    );
    println!(
        "raw_vorticity_range={:.6}..{:.6} smoothed_mean={:.6}",
        vort_stats.min_value, vort_stats.max_value, smooth_stats.mean_value
    );
    println!(
        "frontogenesis_range={:.8}..{:.8} mean={:.8}",
        fronto_stats.min_value, fronto_stats.max_value, fronto_stats.mean_value
    );
    println!("overlay_png={}", overlay.output_path.display());

    Ok(())
}

fn pressure_demo_request(cycle: chrono::DateTime<Utc>) -> HrrrSubsetRequest {
    HrrrSubsetRequest {
        cycle,
        forecast_hour: 0,
        product: "prs".to_string(),
        selections: vec![
            HrrrSelectionRequest {
                variable: "TMP".to_string(),
                level: "850 mb".to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "UGRD".to_string(),
                level: "850 mb".to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "VGRD".to_string(),
                level: "850 mb".to_string(),
                forecast: Some("anl".to_string()),
            },
        ],
    }
}

fn select_field<'a>(
    decoded: &'a [wx_grib::DecodedMessage],
    variable: &str,
    level: &str,
) -> Result<&'a Field2D> {
    decoded
        .iter()
        .find(|message| {
            message.selection.variable == variable
                && message.selection.level.eq_ignore_ascii_case(level)
        })
        .map(|message| &message.field)
        .with_context(|| {
            format!("missing {variable} at {level} in decoded mesoanalysis demo bundle")
        })
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace layout is stable")
        .to_path_buf()
}
