use anyhow::Result;
use wx_fetch::{build_hrrr_plan, FetchRequest};
use wx_grib::{decode_subset, GribSubsetRequest};
use wx_grid::LayerKinematics;
use wx_render::{render_overlay, OverlaySpec};
use wx_severe::compose_significant_tornado_parameter;
use wx_thermo::{compute_surface_parcel, ParcelRequest};
use wx_types::{LevelDescriptor, SoundingLevel, SoundingProfile};

fn main() -> Result<()> {
    let command = std::env::args().nth(1).unwrap_or_else(|| "status".to_string());

    match command.as_str() {
        "status" => {
            println!("rustbox workspace is wired");
            println!("upstream compatibility target: metrust-py");
        }
        "demo" => run_demo()?,
        _ => {
            println!("usage: cargo run -p wx-cli -- [status|demo]");
        }
    }

    Ok(())
}

fn run_demo() -> Result<()> {
    let sounding = SoundingProfile {
        station_id: "KOUN".to_string(),
        levels: vec![
            SoundingLevel {
                pressure_hpa: 1000.0,
                height_m: 300.0,
                temperature_c: 24.0,
                dewpoint_c: 18.0,
                wind_u_ms: 3.0,
                wind_v_ms: 5.0,
            },
            SoundingLevel {
                pressure_hpa: 900.0,
                height_m: 1_000.0,
                temperature_c: 17.0,
                dewpoint_c: 11.0,
                wind_u_ms: 8.0,
                wind_v_ms: 12.0,
            },
        ],
    };

    let levels = [LevelDescriptor {
        name: "surface".to_string(),
        value: 0.0,
        units: "m".to_string(),
    }];

    let plan = build_hrrr_plan(
        &FetchRequest {
            model: "hrrr".to_string(),
            cycle: "20260407-12z".to_string(),
            fields: vec!["cape".to_string()],
        },
        &levels,
    )?;

    let field = decode_subset(
        &plan,
        &GribSubsetRequest {
            variable: "cape".to_string(),
            level: "sfc".to_string(),
        },
    )?;

    let parcel = compute_surface_parcel(&sounding, &ParcelRequest {
        mixed_layer_depth_m: 100.0,
    });
    let severe = compose_significant_tornado_parameter(
        &parcel,
        &LayerKinematics {
            srh_01km_m2s2: 150.0,
            srh_03km_m2s2: 240.0,
            bulk_shear_06km_ms: 18.0,
        },
    );
    let overlay = render_overlay(
        &field,
        &severe,
        &OverlaySpec {
            palette: "cape".to_string(),
            transparent_background: true,
        },
    );

    println!("plan={}", plan.subset_index);
    println!("parcel_mlcape={:.1}", parcel.mlcape_jkg);
    println!("overlay={}", overlay.label);
    Ok(())
}

