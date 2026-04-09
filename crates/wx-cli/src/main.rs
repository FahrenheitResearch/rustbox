use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use std::path::PathBuf;
use wx_fetch::{HrrrSelectionRequest, HrrrSubsetRequest, plan_hrrr_fixture_subset};
use wx_grib::{build_hrrr_sounding_profile, decode_selected_messages};
use wx_render::{OverlaySpec, render_field_to_png};
use wx_severe::compute_significant_tornado_parameter;
use wx_thermo::compute_parcel_diagnostics;

const DEMO_PRESSURE_LEVELS: [&str; 7] = [
    "1000 mb", "925 mb", "850 mb", "700 mb", "500 mb", "400 mb", "300 mb",
];
const DEMO_PROFILE_X: usize = 1_798;
const DEMO_PROFILE_Y: usize = 1_058;

fn main() -> Result<()> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "status".to_string());

    match command.as_str() {
        "status" => print_status(),
        "demo" => run_demo()?,
        _ => {
            println!("usage: cargo run -p wx-cli -- [status|demo]");
        }
    }

    Ok(())
}

fn print_status() {
    println!(
        "wx-fetch: real HRRR .idx subset planning with explicit source-object vs fixture-fragment byte ranges"
    );
    println!(
        "wx-grib: real GRIB2 decode for scalar and multi-message HRRR fixtures with bundle checks"
    );
    println!(
        "wx-grib: real HRRR column extraction into SoundingProfile at a fixed fixture grid point"
    );
    println!("wx-thermo: real sharprs-derived SBCAPE/MLCAPE/MUCAPE/CIN diagnostics");
    println!(
        "wx-severe: real fixed-layer STP and exact-layer kinematics via a local sharprs compatibility fork"
    );
    println!("wx-render: real transparent PNG overlay writer");
    println!("wx-cuda: stub capability surface only");
    println!("wx-radar/wx-wrf/wx-zarr/wx-py: not implemented in this milestone");
}

fn run_demo() -> Result<()> {
    let cycle = Utc
        .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
        .single()
        .expect("valid fixture cycle");
    let fixture_root = repo_root().join("tests/fixtures");
    let (x, y) = (DEMO_PROFILE_X, DEMO_PROFILE_Y);

    let surface_fragment = fixture_root.join("hrrr_demo_surface_fragment.grib2");
    let surface_plan = plan_hrrr_fixture_subset(
        &surface_request(cycle),
        &std::fs::read_to_string(fixture_root.join("hrrr_demo_surface_fragment.idx"))?,
        std::fs::metadata(&surface_fragment)?.len(),
    )?;
    let pressure_fragment = fixture_root.join("hrrr_demo_pressure_fragment.grib2");
    let pressure_plan = plan_hrrr_fixture_subset(
        &pressure_request(cycle),
        &std::fs::read_to_string(fixture_root.join("hrrr_demo_pressure_fragment.idx"))?,
        std::fs::metadata(&pressure_fragment)?.len(),
    )?;

    let surface_messages = decode_selected_messages(&surface_fragment, &surface_plan)?;
    let pressure_messages = decode_selected_messages(&pressure_fragment, &pressure_plan)?;
    let sounding = build_hrrr_sounding_profile(&surface_messages, &pressure_messages, x, y)?;
    let parcel = compute_parcel_diagnostics(&sounding)?;
    let severe = compute_significant_tornado_parameter(&sounding, &parcel)?;
    let overlay_field = surface_messages
        .first()
        .map(|message| message.field.clone())
        .context("surface fixture decode did not return the requested gust field")?;

    let output_path = repo_root().join("target/demo/hrrr_gust_surface_overlay.png");
    let overlay = render_field_to_png(
        &overlay_field,
        &OverlaySpec {
            palette: "winds".to_string(),
            transparent_background: true,
            value_range: None,
        },
        &output_path,
    )?;

    let selection = &surface_plan.selections[0];
    let (field_min, field_max) = overlay_field
        .finite_min_max()
        .expect("decoded field should have data");

    println!(
        "selection_msg={} fragment_bytes={}..{} source_range_origin={}",
        selection.message_number,
        selection.start,
        selection.end_exclusive,
        surface_plan.byte_range_origin.label()
    );
    println!("surface_source_grib_url={}", surface_plan.source_grib_url);
    println!("pressure_source_grib_url={}", pressure_plan.source_grib_url);
    println!(
        "field={} level={} grid={}x{} range={:.2}..{:.2}",
        overlay_field.metadata.parameter,
        overlay_field.metadata.level.description,
        overlay_field.grid.nx,
        overlay_field.grid.ny,
        field_min,
        field_max
    );
    println!(
        "profile_point=x{} y{} levels={} sfc_p={:.1} top_p={:.1}",
        x,
        y,
        sounding.levels.len(),
        sounding
            .levels
            .first()
            .map(|level| level.pressure_hpa)
            .unwrap_or(0.0),
        sounding
            .levels
            .last()
            .map(|level| level.pressure_hpa)
            .unwrap_or(0.0)
    );
    println!(
        "sbcape={:.1} sbcin={:.1} mlcape={:.1} mlcin={:.1} mucape={:.1} mucin={:.1}",
        parcel.surface.cape_jkg,
        parcel.surface.cin_jkg,
        parcel.mixed_layer.cape_jkg,
        parcel.mixed_layer.cin_jkg,
        parcel.most_unstable.cape_jkg,
        parcel.most_unstable.cin_jkg
    );
    println!(
        "srh01={:.1} srh03={:.1} shear06={:.2} stp_fixed={:.2}",
        severe.kinematics.srh_01km_m2s2,
        severe.kinematics.srh_03km_m2s2,
        severe.kinematics.bulk_shear_06km_ms,
        severe.significant_tornado_parameter
    );
    println!("overlay_png={}", overlay.output_path.display());
    Ok(())
}

fn surface_request(cycle: chrono::DateTime<Utc>) -> HrrrSubsetRequest {
    HrrrSubsetRequest {
        cycle,
        forecast_hour: 0,
        product: "sfc".to_string(),
        selections: vec![
            HrrrSelectionRequest {
                variable: "GUST".to_string(),
                level: "surface".to_string(),
                forecast: Some("anl".to_string()),
            },
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
    }
}

fn pressure_request(cycle: chrono::DateTime<Utc>) -> HrrrSubsetRequest {
    HrrrSubsetRequest {
        cycle,
        forecast_hour: 0,
        product: "prs".to_string(),
        selections: DEMO_PRESSURE_LEVELS
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
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace layout is stable")
        .to_path_buf()
}
