use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, TimeZone, Utc};
use grib_core::grib2::{
    Grib2File, level_name, parameter_name, parameter_units, unpack_message_normalized,
};
use std::path::Path;
use wx_fetch::{SubsetMessageRef, SubsetPlan};
use wx_types::{
    CoordinateMetadata, Field2D, FieldMetadata, GridSpec, LevelMetadata, ProjectionKind,
    SoundingLevel, SoundingProfile, ValidTimeMetadata,
};

const MS_TO_KTS: f64 = 1.943_844_492_440_604_6;
const KELVIN_OFFSET_C: f64 = 273.15;
const REQUIRED_PRESSURE_LEVELS: [&str; 7] = [
    "1000 mb", "925 mb", "850 mb", "700 mb", "500 mb", "400 mb", "300 mb",
];

#[derive(Debug, Clone)]
pub struct DecodedMessage {
    pub selection: SubsetMessageRef,
    pub field: Field2D,
}

pub fn decode_selected_message(fragment_path: &Path, plan: &SubsetPlan) -> Result<Field2D> {
    Ok(decode_selected_messages(fragment_path, plan)?
        .into_iter()
        .next()
        .context("subset plan did not contain any selections")?
        .field)
}

pub fn decode_selected_messages(
    fragment_path: &Path,
    plan: &SubsetPlan,
) -> Result<Vec<DecodedMessage>> {
    let bytes = std::fs::read(fragment_path)
        .with_context(|| format!("failed to read GRIB fixture {}", fragment_path.display()))?;
    let mut decoded = Vec::with_capacity(plan.selections.len());

    for selection in &plan.selections {
        decoded.push(DecodedMessage {
            selection: selection.clone(),
            field: decode_message_bytes(&bytes, selection, plan)?,
        });
    }

    Ok(decoded)
}

pub fn find_valid_hrrr_profile_point(
    surface_messages: &[DecodedMessage],
    pressure_messages: &[DecodedMessage],
) -> Result<(usize, usize)> {
    let reference_field = surface_messages
        .first()
        .or_else(|| pressure_messages.first())
        .map(|message| &message.field)
        .context("no decoded fields available to search for a valid HRRR column")?;

    for y in (0..reference_field.grid.ny).rev() {
        for x in (0..reference_field.grid.nx).rev() {
            if column_levels(surface_messages, pressure_messages, x, y).is_ok() {
                return Ok((x, y));
            }
        }
    }

    bail!("failed to find a valid HRRR model column in the decoded fixtures")
}

pub fn build_hrrr_sounding_profile(
    surface_messages: &[DecodedMessage],
    pressure_messages: &[DecodedMessage],
    x: usize,
    y: usize,
) -> Result<SoundingProfile> {
    let levels = column_levels(surface_messages, pressure_messages, x, y)?;
    let valid_time = surface_messages
        .first()
        .or_else(|| pressure_messages.first())
        .map(|message| message.field.metadata.valid.valid_time);

    Ok(SoundingProfile {
        station_id: format!("hrrr_x{}_y{}", x, y),
        latitude: None,
        longitude: None,
        valid_time,
        levels,
    })
}

fn decode_message_bytes(
    bytes: &[u8],
    selection: &SubsetMessageRef,
    plan: &SubsetPlan,
) -> Result<Field2D> {
    let selection = plan
        .selections
        .iter()
        .find(|candidate| {
            candidate.message_number == selection.message_number
                && candidate.start == selection.start
                && candidate.end_exclusive == selection.end_exclusive
        })
        .unwrap_or(selection);
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
            short_name: selection.variable.clone(),
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
                value: Some(canonical_level_value(
                    message.product.level_type,
                    message.product.level_value,
                )),
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

fn column_levels(
    surface_messages: &[DecodedMessage],
    pressure_messages: &[DecodedMessage],
    x: usize,
    y: usize,
) -> Result<Vec<SoundingLevel>> {
    let surface_pressure_hpa = value_at(
        &find_message(surface_messages, "PRES", "surface")?.field,
        x,
        y,
    )? as f64
        / 100.0;
    let surface_height_m = value_at(
        &find_message(surface_messages, "HGT", "surface")?.field,
        x,
        y,
    )? as f64;
    let surface_temperature_c = kelvin_to_celsius(value_at(
        &find_message(surface_messages, "TMP", "2 m above ground")?.field,
        x,
        y,
    )? as f64);
    let surface_dewpoint_c = kelvin_to_celsius(value_at(
        &find_message(surface_messages, "DPT", "2 m above ground")?.field,
        x,
        y,
    )? as f64);
    let surface_u_ms = value_at(
        &find_message(surface_messages, "UGRD", "10 m above ground")?.field,
        x,
        y,
    )? as f64;
    let surface_v_ms = value_at(
        &find_message(surface_messages, "VGRD", "10 m above ground")?.field,
        x,
        y,
    )? as f64;
    let (surface_wdir, surface_wspd_kts) = wind_direction_speed_from_uv(surface_u_ms, surface_v_ms);

    let mut levels = vec![SoundingLevel {
        pressure_hpa: surface_pressure_hpa,
        height_m: surface_height_m,
        temperature_c: surface_temperature_c,
        dewpoint_c: surface_dewpoint_c,
        wind_direction_deg: surface_wdir,
        wind_speed_kts: surface_wspd_kts,
    }];

    let mut previous_height = surface_height_m;
    for level_name in REQUIRED_PRESSURE_LEVELS {
        let height_m = value_at(
            &find_message(pressure_messages, "HGT", level_name)?.field,
            x,
            y,
        )? as f64;
        let temperature_c = kelvin_to_celsius(value_at(
            &find_message(pressure_messages, "TMP", level_name)?.field,
            x,
            y,
        )? as f64);
        let dewpoint_c = kelvin_to_celsius(value_at(
            &find_message(pressure_messages, "DPT", level_name)?.field,
            x,
            y,
        )? as f64);
        let u_ms = value_at(
            &find_message(pressure_messages, "UGRD", level_name)?.field,
            x,
            y,
        )? as f64;
        let v_ms = value_at(
            &find_message(pressure_messages, "VGRD", level_name)?.field,
            x,
            y,
        )? as f64;
        let (wind_direction_deg, wind_speed_kts) = wind_direction_speed_from_uv(u_ms, v_ms);

        let pressure_hpa = find_message(pressure_messages, "TMP", level_name)?
            .field
            .metadata
            .level
            .value
            .context("pressure-level GRIB field did not expose a level value")?;

        if pressure_hpa >= surface_pressure_hpa || height_m <= previous_height {
            continue;
        }

        levels.push(SoundingLevel {
            pressure_hpa,
            height_m,
            temperature_c,
            dewpoint_c,
            wind_direction_deg,
            wind_speed_kts,
        });
        previous_height = height_m;
    }

    if levels.len() < 7 {
        bail!(
            "HRRR column at grid point {},{} did not retain enough above-ground levels",
            x,
            y
        );
    }
    if levels
        .last()
        .map(|level| level.height_m - surface_height_m)
        .unwrap_or(0.0)
        < 6_000.0
    {
        bail!("HRRR column does not extend high enough for 0-6 km kinematics");
    }

    Ok(levels)
}

fn find_message<'a>(
    messages: &'a [DecodedMessage],
    variable: &str,
    level: &str,
) -> Result<&'a DecodedMessage> {
    messages
        .iter()
        .find(|message| {
            message.selection.variable.eq_ignore_ascii_case(variable)
                && message.selection.level.eq_ignore_ascii_case(level)
        })
        .with_context(|| format!("missing decoded message for {} {}", variable, level))
}

fn value_at(field: &Field2D, x: usize, y: usize) -> Result<f32> {
    if x >= field.grid.nx || y >= field.grid.ny {
        bail!(
            "grid point {},{} is outside field dimensions {}x{}",
            x,
            y,
            field.grid.nx,
            field.grid.ny
        );
    }

    let index = y
        .checked_mul(field.grid.nx)
        .and_then(|offset| offset.checked_add(x))
        .context("grid indexing overflowed")?;
    let value = field
        .values
        .get(index)
        .copied()
        .context("grid point index was outside the decoded values")?;
    if !value.is_finite() {
        bail!(
            "decoded field {} {} has a non-finite value at grid point {},{}",
            field.metadata.short_name,
            field.metadata.level.description,
            x,
            y
        );
    }

    Ok(value)
}

fn kelvin_to_celsius(value_k: f64) -> f64 {
    value_k - KELVIN_OFFSET_C
}

fn wind_direction_speed_from_uv(u_ms: f64, v_ms: f64) -> (f64, f64) {
    let u_kts = u_ms * MS_TO_KTS;
    let v_kts = v_ms * MS_TO_KTS;
    let speed = u_kts.hypot(v_kts);
    if speed <= f64::EPSILON {
        return (0.0, 0.0);
    }

    let mut direction = (-u_kts).atan2(-v_kts).to_degrees();
    if direction < 0.0 {
        direction += 360.0;
    }
    (direction, speed)
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

fn canonical_level_value(level_type: u8, raw_value: f64) -> f64 {
    match level_type {
        100 => raw_value / 100.0,
        _ => raw_value,
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
    use wx_fetch::{
        HrrrSelectionRequest, HrrrSubsetRequest, plan_hrrr_subset, plan_hrrr_subset_with_length,
    };
    use wx_severe::compute_significant_tornado_parameter;
    use wx_thermo::compute_parcel_diagnostics;
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
                selections: vec![HrrrSelectionRequest {
                    variable: "GUST".to_string(),
                    level: "surface".to_string(),
                }],
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

    #[test]
    fn decode_fragment_returns_multiple_selected_messages() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_demo_surface_fragment.idx"))
            .expect("fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let fragment_len = std::fs::metadata(fixture_path("hrrr_demo_surface_fragment.grib2"))
            .expect("fixture should exist")
            .len();
        let plan = plan_hrrr_subset_with_length(
            &HrrrSubsetRequest {
                cycle,
                forecast_hour: 0,
                product: "sfc".to_string(),
                selections: vec![
                    HrrrSelectionRequest {
                        variable: "GUST".to_string(),
                        level: "surface".to_string(),
                    },
                    HrrrSelectionRequest {
                        variable: "PRES".to_string(),
                        level: "surface".to_string(),
                    },
                    HrrrSelectionRequest {
                        variable: "TMP".to_string(),
                        level: "2 m above ground".to_string(),
                    },
                ],
            },
            &idx_text,
            Some(fragment_len),
        )
        .expect("plan should succeed");

        let decoded =
            decode_selected_messages(&fixture_path("hrrr_demo_surface_fragment.grib2"), &plan)
                .expect("decode should succeed");

        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].selection.variable, "GUST");
        assert_eq!(decoded[1].selection.variable, "PRES");
        assert_eq!(decoded[2].selection.level, "2 m above ground");
        assert_eq!(
            decoded[2].field.values.len(),
            decoded[2].field.expected_len()
        );
    }

    #[test]
    fn extracted_hrrr_column_builds_valid_model_profile() {
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");

        let surface_plan = plan_hrrr_subset_with_length(
            &HrrrSubsetRequest {
                cycle,
                forecast_hour: 0,
                product: "sfc".to_string(),
                selections: vec![
                    HrrrSelectionRequest {
                        variable: "GUST".to_string(),
                        level: "surface".to_string(),
                    },
                    HrrrSelectionRequest {
                        variable: "PRES".to_string(),
                        level: "surface".to_string(),
                    },
                    HrrrSelectionRequest {
                        variable: "HGT".to_string(),
                        level: "surface".to_string(),
                    },
                    HrrrSelectionRequest {
                        variable: "TMP".to_string(),
                        level: "2 m above ground".to_string(),
                    },
                    HrrrSelectionRequest {
                        variable: "DPT".to_string(),
                        level: "2 m above ground".to_string(),
                    },
                    HrrrSelectionRequest {
                        variable: "UGRD".to_string(),
                        level: "10 m above ground".to_string(),
                    },
                    HrrrSelectionRequest {
                        variable: "VGRD".to_string(),
                        level: "10 m above ground".to_string(),
                    },
                ],
            },
            &std::fs::read_to_string(fixture_path("hrrr_demo_surface_fragment.idx"))
                .expect("surface idx should be readable"),
            Some(
                std::fs::metadata(fixture_path("hrrr_demo_surface_fragment.grib2"))
                    .expect("surface fragment should exist")
                    .len(),
            ),
        )
        .expect("surface plan should succeed");
        let pressure_plan = plan_hrrr_subset_with_length(
            &HrrrSubsetRequest {
                cycle,
                forecast_hour: 0,
                product: "prs".to_string(),
                selections: REQUIRED_PRESSURE_LEVELS
                    .into_iter()
                    .flat_map(|level| {
                        [
                            HrrrSelectionRequest {
                                variable: "HGT".to_string(),
                                level: level.to_string(),
                            },
                            HrrrSelectionRequest {
                                variable: "TMP".to_string(),
                                level: level.to_string(),
                            },
                            HrrrSelectionRequest {
                                variable: "DPT".to_string(),
                                level: level.to_string(),
                            },
                            HrrrSelectionRequest {
                                variable: "UGRD".to_string(),
                                level: level.to_string(),
                            },
                            HrrrSelectionRequest {
                                variable: "VGRD".to_string(),
                                level: level.to_string(),
                            },
                        ]
                    })
                    .collect(),
            },
            &std::fs::read_to_string(fixture_path("hrrr_demo_pressure_fragment.idx"))
                .expect("pressure idx should be readable"),
            Some(
                std::fs::metadata(fixture_path("hrrr_demo_pressure_fragment.grib2"))
                    .expect("pressure fragment should exist")
                    .len(),
            ),
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

        let (x, y) = find_valid_hrrr_profile_point(&surface_messages, &pressure_messages)
            .expect("valid point");
        let profile = build_hrrr_sounding_profile(&surface_messages, &pressure_messages, x, y)
            .expect("profile extraction should succeed");

        assert!(profile.levels.len() >= 8);
        assert!(profile.valid_time.is_some());
        assert!(profile.levels[0].pressure_hpa > profile.levels[1].pressure_hpa);
        assert!(
            profile
                .levels
                .windows(2)
                .all(|pair| pair[0].pressure_hpa > pair[1].pressure_hpa)
        );
        assert!(
            profile
                .levels
                .windows(2)
                .all(|pair| pair[0].height_m < pair[1].height_m)
        );
        assert!(
            profile
                .levels
                .iter()
                .all(|level| level.temperature_c.is_finite() && level.dewpoint_c.is_finite())
        );

        let parcel = compute_parcel_diagnostics(&profile).expect("parcel diagnostics should work");
        assert!(parcel.surface.cape_jkg.is_finite());
        assert!(parcel.surface.cin_jkg.is_finite());

        let severe = compute_significant_tornado_parameter(&profile, &parcel)
            .expect("severe diagnostics should work");
        assert!(severe.significant_tornado_parameter.is_finite());
        assert!(severe.kinematics.bulk_shear_06km_ms.is_finite());
    }
}
