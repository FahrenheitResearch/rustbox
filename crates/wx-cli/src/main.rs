use anyhow::Result;
use chrono::{TimeZone, Utc};
use std::path::{Path, PathBuf};
use wx_fetch::{HrrrSubsetRequest, plan_hrrr_subset};
use wx_grib::decode_selected_message;
use wx_render::{OverlaySpec, render_field_to_png};
use wx_severe::compute_significant_tornado_parameter;
use wx_thermo::compute_parcel_diagnostics;
use wx_types::SoundingProfile;

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
    println!("wx-fetch: real HRRR .idx subset planning from fixture-backed manifests");
    println!("wx-grib: real GRIB2 decode for one HRRR scalar message");
    println!("wx-thermo: real sharprs-derived SBCAPE/MLCAPE/MUCAPE/CIN diagnostics");
    println!("wx-severe: real sharprs-derived STP fixed and kinematic inputs");
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
    let idx_text = std::fs::read_to_string(fixture_root.join("hrrr_gust_surface_fragment.idx"))?;
    let plan = plan_hrrr_subset(
        &HrrrSubsetRequest {
            cycle,
            forecast_hour: 0,
            product: "sfc".to_string(),
            variable: "GUST".to_string(),
            level: "surface".to_string(),
        },
        &idx_text,
    )?;
    let field = decode_selected_message(
        &fixture_root.join("hrrr_gust_surface_fragment.grib2"),
        &plan,
    )?;

    let sounding = load_sounding_fixture(&fixture_root.join("sounding_supercell.json"))?;
    let parcel = compute_parcel_diagnostics(&sounding)?;
    let severe = compute_significant_tornado_parameter(&sounding, &parcel)?;

    let output_path = repo_root().join("target/demo/hrrr_gust_surface_overlay.png");
    let overlay = render_field_to_png(
        &field,
        &OverlaySpec {
            palette: "winds".to_string(),
            transparent_background: true,
            value_range: None,
        },
        &output_path,
    )?;

    let selection = &plan.selections[0];
    let (field_min, field_max) = field
        .finite_min_max()
        .expect("decoded field should have data");

    println!(
        "selection_msg={} bytes={}..{}",
        selection.message_number, selection.start, selection.end_exclusive
    );
    println!("grib_url={}", plan.grib_url);
    println!(
        "field={} level={} grid={}x{} range={:.2}..{:.2}",
        field.metadata.parameter,
        field.metadata.level.description,
        field.grid.nx,
        field.grid.ny,
        field_min,
        field_max
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

fn load_sounding_fixture(path: &Path) -> Result<SoundingProfile> {
    let text = std::fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&text)?;
    serde_json::from_value(json).map_err(Into::into)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace layout is stable")
        .to_path_buf()
}
