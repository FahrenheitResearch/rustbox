use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, TimeZone, Utc};
use grib_core::grib2::{
    Grib2File, level_name, parameter_name, parameter_units, unpack_message_normalized,
};
use std::path::Path;
use wx_fetch::SubsetPlan;
use wx_types::{
    CoordinateMetadata, Field2D, FieldMetadata, GridSpec, LevelMetadata, ProjectionKind,
    ValidTimeMetadata,
};

pub fn decode_selected_message(fragment_path: &Path, plan: &SubsetPlan) -> Result<Field2D> {
    let selection = plan
        .selections
        .first()
        .context("subset plan did not contain any selections")?;

    let bytes = std::fs::read(fragment_path)
        .with_context(|| format!("failed to read GRIB fixture {}", fragment_path.display()))?;
    let start = usize::try_from(selection.start).context("subset start offset overflowed usize")?;
    let end =
        usize::try_from(selection.end_exclusive).context("subset end offset overflowed usize")?;

    if end <= start || end > bytes.len() {
        bail!(
            "invalid selection range {}..{} for fragment length {}",
            start,
            end,
            bytes.len()
        );
    }

    let file = Grib2File::from_bytes(&bytes[start..end])
        .context("failed to parse selected GRIB2 bytes")?;
    let message = file
        .messages
        .first()
        .context("selected GRIB2 bytes did not contain any messages")?;
    let decoded = unpack_message_normalized(message).context("failed to unpack GRIB2 message")?;

    let values: Vec<f32> = decoded.iter().map(|value| *value as f32).collect();
    let grid = GridSpec {
        nx: message.grid.nx as usize,
        ny: message.grid.ny as usize,
        projection: projection_from_template(
            message.grid.template,
            message.grid.latin1,
            message.grid.latin2,
            message.grid.lov,
            message.grid.lad,
        ),
        coordinates: CoordinateMetadata {
            lat1: message.grid.lat1,
            lon1: message.grid.lon1,
            lat2: message.grid.lat2,
            lon2: message.grid.lon2,
            dx: message.grid.dx,
            dy: message.grid.dy,
        },
        scan_mode: message.grid.scan_mode,
    };

    if values.len() != grid.nx * grid.ny {
        bail!(
            "decoded value count {} does not match grid {}x{}",
            values.len(),
            grid.nx,
            grid.ny
        );
    }

    Ok(Field2D {
        metadata: FieldMetadata {
            parameter: parameter_name(
                message.discipline,
                message.product.parameter_category,
                message.product.parameter_number,
            )
            .to_string(),
            units: parameter_units(
                message.discipline,
                message.product.parameter_category,
                message.product.parameter_number,
            )
            .to_string(),
            level: LevelMetadata {
                code: message.product.level_type,
                description: canonical_level_description(
                    message.product.level_type,
                    level_name(message.product.level_type),
                ),
                value: Some(message.product.level_value),
                units: level_units(message.product.level_type).to_string(),
            },
            source: plan.source.clone(),
            run: plan.run.clone(),
            valid: ValidTimeMetadata {
                reference_time: utc_from_naive(message.reference_time),
                valid_time: compute_valid_time(
                    utc_from_naive(message.reference_time),
                    message.product.forecast_time,
                    message.product.time_range_unit,
                ),
            },
        },
        grid,
        values,
    })
}

fn utc_from_naive(value: chrono::NaiveDateTime) -> DateTime<Utc> {
    Utc.from_utc_datetime(&value)
}

fn compute_valid_time(
    reference_time: DateTime<Utc>,
    forecast_time: u32,
    unit: u8,
) -> DateTime<Utc> {
    let duration = match unit {
        0 => Duration::minutes(i64::from(forecast_time)),
        1 => Duration::hours(i64::from(forecast_time)),
        2 => Duration::days(i64::from(forecast_time)),
        _ => Duration::zero(),
    };
    reference_time + duration
}

fn level_units(level_type: u8) -> &'static str {
    match level_type {
        100 => "hPa",
        103 => "m",
        _ => "",
    }
}

fn canonical_level_description(level_type: u8, default_name: &str) -> String {
    match level_type {
        1 => "surface".to_string(),
        _ => default_name.to_string(),
    }
}

fn projection_from_template(
    template: u16,
    latin1: f64,
    latin2: f64,
    lov: f64,
    lad: f64,
) -> ProjectionKind {
    match template {
        0 => ProjectionKind::LatitudeLongitude,
        10 => ProjectionKind::Mercator { lad },
        20 => ProjectionKind::PolarStereographic { lad, lov },
        30 => ProjectionKind::LambertConformal {
            latin1,
            latin2,
            lov,
        },
        _ => ProjectionKind::Unknown { template },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::path::PathBuf;
    use wx_fetch::{HrrrSubsetRequest, plan_hrrr_subset};
    use wx_types::ProjectionKind;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[test]
    fn decode_fragment_returns_expected_grid_and_metadata() {
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
                variable: "GUST".to_string(),
                level: "surface".to_string(),
            },
            &idx_text,
        )
        .expect("plan should succeed");

        let field =
            decode_selected_message(&fixture_path("hrrr_gust_surface_fragment.grib2"), &plan)
                .expect("decode should succeed");

        assert_eq!(field.grid.nx, 1_799);
        assert_eq!(field.grid.ny, 1_059);
        assert!(matches!(
            field.grid.projection,
            ProjectionKind::LambertConformal { .. }
        ));
        assert_eq!(field.metadata.source.model, "hrrr");
        assert_eq!(field.metadata.level.code, 1);
        assert_eq!(field.metadata.level.description, "surface");
        assert!(
            field.metadata.parameter.to_lowercase().contains("gust"),
            "expected gust metadata, got {}",
            field.metadata.parameter
        );
        assert_eq!(field.values.len(), field.grid.nx * field.grid.ny);
        assert!(field.finite_min_max().is_some());
    }
}
