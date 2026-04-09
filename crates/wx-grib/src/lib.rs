use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, TimeZone, Utc};
use grib_core::grib2::{
    Grib2File, level_name, parameter_name, parameter_units, unpack_message_normalized,
};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;
use wx_fetch::{StagedSubset, SubsetMessageRef, SubsetPlan};
use wx_types::{
    CoordinateMetadata, Field2D, Field2DSummary, Field3D, Field3DSummary, FieldBundle,
    FieldBundleSummary, FieldMetadata, GridSpec, LevelAxis, LevelMetadata, NativeVolume,
    ProjectionKind, RunMetadata, SoundingLevel, SoundingProfile, TimeAxis, ValidTimeMetadata,
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

pub fn decode_staged_subset(staged_subset: &StagedSubset) -> Result<Vec<DecodedMessage>> {
    decode_selected_messages(&staged_subset.local_grib_path, &staged_subset.local_plan)
}

pub fn decode_field_bundle(fragment_path: &Path, plan: &SubsetPlan) -> Result<FieldBundle> {
    let decoded = decode_selected_messages(fragment_path, plan)?;
    build_field_bundle(&decoded)
}

pub fn decode_native_volume(
    fragment_path: &Path,
    plan: &SubsetPlan,
    product: &str,
) -> Result<NativeVolume> {
    let bundle = decode_field_bundle(fragment_path, plan)?;
    Ok(NativeVolume {
        product: product.to_string(),
        time_axis: TimeAxis {
            cycles: vec![bundle.run.cycle],
            valid_times: vec![bundle.valid.valid_time],
            forecast_hours: vec![bundle.run.forecast_hour],
        },
        bundle,
    })
}

pub fn build_field_bundle(messages: &[DecodedMessage]) -> Result<FieldBundle> {
    validate_bundle(messages, "decoded")?;

    let first = messages
        .first()
        .context("decoded message bundle was empty")?;
    let fields_2d: Vec<Field2D> = messages
        .iter()
        .map(|message| message.field.clone())
        .collect();
    let fields_3d = build_stacked_fields(messages)?;

    Ok(FieldBundle {
        source: first.field.metadata.source.clone(),
        run: first.field.metadata.run.clone(),
        valid: first.field.metadata.valid.clone(),
        grid: first.field.grid.clone(),
        fields_2d,
        fields_3d,
    })
}

pub fn summarize_field_bundle(bundle: &FieldBundle) -> FieldBundleSummary {
    FieldBundleSummary {
        source: bundle.source.clone(),
        run: bundle.run.clone(),
        valid: bundle.valid.clone(),
        grid: bundle.grid.clone(),
        fields_2d: bundle
            .fields_2d
            .iter()
            .map(|field| {
                let (finite_min, finite_max) = match field.finite_min_max() {
                    Some((min_value, max_value)) => (Some(min_value), Some(max_value)),
                    None => (None, None),
                };
                Field2DSummary {
                    short_name: field.metadata.short_name.clone(),
                    parameter: field.metadata.parameter.clone(),
                    level: field.metadata.level.description.clone(),
                    units: field.metadata.units.clone(),
                    nx: field.grid.nx,
                    ny: field.grid.ny,
                    finite_min,
                    finite_max,
                }
            })
            .collect(),
        fields_3d: bundle
            .fields_3d
            .iter()
            .map(|field| {
                let (finite_min, finite_max) = match field.finite_min_max() {
                    Some((min_value, max_value)) => (Some(min_value), Some(max_value)),
                    None => (None, None),
                };
                Field3DSummary {
                    short_name: field.metadata.short_name.clone(),
                    parameter: field.metadata.parameter.clone(),
                    units: field.metadata.units.clone(),
                    level_count: field.level_axis.levels.len(),
                    levels: field
                        .level_axis
                        .levels
                        .iter()
                        .map(|level| level.description.clone())
                        .collect(),
                    nx: field.grid.nx,
                    ny: field.grid.ny,
                    finite_min,
                    finite_max,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
fn find_valid_hrrr_profile_point(
    surface_messages: &[DecodedMessage],
    pressure_messages: &[DecodedMessage],
) -> Result<(usize, usize)> {
    validate_bundle(surface_messages, "surface")?;
    validate_bundle(pressure_messages, "pressure")?;
    validate_compatible_bundles(surface_messages, pressure_messages)?;

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
    validate_bundle(surface_messages, "surface")?;
    validate_bundle(pressure_messages, "pressure")?;
    validate_compatible_bundles(surface_messages, pressure_messages)?;

    let levels = column_levels(surface_messages, pressure_messages, x, y)?;
    let reference_message = surface_messages
        .first()
        .or_else(|| pressure_messages.first())
        .context("no decoded fields available to build an HRRR sounding profile")?;

    Ok(SoundingProfile {
        station_id: format!(
            "hrrr_conus_f{:02}_x{}_y{}",
            reference_message.field.metadata.run.forecast_hour, x, y
        ),
        latitude: None,
        longitude: None,
        grid_x: Some(x),
        grid_y: Some(y),
        valid_time: Some(reference_message.field.metadata.valid.valid_time),
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
    if file.messages.len() != 1 {
        bail!(
            "expected exactly one GRIB2 message in selected byte range {}..{}, found {}",
            selection.start,
            selection.end_exclusive,
            file.messages.len()
        );
    }
    let message = file
        .messages
        .first()
        .context("selected GRIB2 bytes did not contain any messages")?;
    validate_selection_matches_message(selection, message)?;
    let reference_time = utc_from_naive(message.reference_time);
    let forecast_duration = forecast_duration(
        message.product.forecast_time,
        message.product.time_range_unit,
    )?;
    let valid_time = reference_time + forecast_duration;
    let forecast_hour =
        forecast_hour_from_duration(forecast_duration, message.product.time_range_unit)?;
    validate_plan_matches_message(plan, selection, reference_time, forecast_hour, valid_time)?;
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
            short_name: decoded_short_name(
                message.discipline,
                message.product.parameter_category,
                message.product.parameter_number,
            )
            .unwrap_or(selection.variable.as_str())
            .to_string(),
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
                    message.product.level_value,
                    level_name(message.product.level_type),
                ),
                value: Some(canonical_level_value(
                    message.product.level_type,
                    message.product.level_value,
                )),
                units: level_units(message.product.level_type).to_string(),
            },
            source: plan.source.clone(),
            run: RunMetadata {
                cycle: reference_time,
                forecast_hour,
            },
            valid: ValidTimeMetadata {
                reference_time,
                valid_time,
            },
        },
        grid,
        values,
    })
}

fn validate_bundle(messages: &[DecodedMessage], bundle_name: &str) -> Result<()> {
    let first = messages
        .first()
        .with_context(|| format!("{bundle_name} decoded message bundle was empty"))?;

    for message in &messages[1..] {
        if message.field.grid != first.field.grid {
            bail!("{bundle_name} bundle mixes incompatible grid geometry");
        }
        if message.field.metadata.run != first.field.metadata.run {
            bail!("{bundle_name} bundle mixes incompatible run metadata");
        }
        if message.field.metadata.valid != first.field.metadata.valid {
            bail!("{bundle_name} bundle mixes incompatible valid/reference times");
        }
        if message.field.metadata.source != first.field.metadata.source {
            bail!("{bundle_name} bundle mixes incompatible source metadata");
        }
    }

    Ok(())
}

fn validate_compatible_bundles(
    surface_messages: &[DecodedMessage],
    pressure_messages: &[DecodedMessage],
) -> Result<()> {
    let surface = surface_messages
        .first()
        .context("surface decoded message bundle was empty")?;
    let pressure = pressure_messages
        .first()
        .context("pressure decoded message bundle was empty")?;

    if surface.field.grid != pressure.field.grid {
        bail!("surface and pressure bundles do not share the same HRRR grid geometry");
    }
    if surface.field.metadata.run != pressure.field.metadata.run {
        bail!("surface and pressure bundles do not share the same HRRR run metadata");
    }
    if surface.field.metadata.valid != pressure.field.metadata.valid {
        bail!("surface and pressure bundles do not share the same valid/reference times");
    }
    if surface.field.metadata.source.provider != pressure.field.metadata.source.provider
        || surface.field.metadata.source.model != pressure.field.metadata.source.model
    {
        bail!("surface and pressure bundles do not share the same source provenance");
    }

    Ok(())
}

fn build_stacked_fields(messages: &[DecodedMessage]) -> Result<Vec<Field3D>> {
    let mut groups: BTreeMap<(String, String, String, u8, String), Vec<Field2D>> = BTreeMap::new();
    for message in messages {
        let key = (
            message.field.metadata.short_name.clone(),
            message.field.metadata.parameter.clone(),
            message.field.metadata.units.clone(),
            message.field.metadata.level.code,
            message.field.metadata.level.units.clone(),
        );
        groups.entry(key).or_default().push(message.field.clone());
    }

    let mut stacked = Vec::new();
    for ((_short_name, _parameter, _units, _level_code, _level_units), mut fields) in groups {
        let distinct_levels = fields
            .iter()
            .filter_map(|field| field.metadata.level.value)
            .collect::<Vec<_>>();
        if fields.len() < 2 || distinct_levels.len() < 2 {
            continue;
        }

        fields.sort_by(|left, right| {
            compare_level_metadata(&left.metadata.level, &right.metadata.level)
        });
        let first = fields
            .first()
            .context("stacked field grouping unexpectedly empty")?;
        let mut values = Vec::with_capacity(fields.len() * first.expected_len());
        let levels: Vec<LevelMetadata> = fields
            .iter()
            .map(|field| field.metadata.level.clone())
            .collect();
        for field in &fields {
            values.extend(field.values.iter().copied());
        }

        stacked.push(Field3D {
            metadata: FieldMetadata {
                short_name: first.metadata.short_name.clone(),
                parameter: first.metadata.parameter.clone(),
                units: first.metadata.units.clone(),
                level: LevelMetadata {
                    code: first.metadata.level.code,
                    description: format!("stacked {}", first.metadata.level.units),
                    value: None,
                    units: first.metadata.level.units.clone(),
                },
                source: first.metadata.source.clone(),
                run: first.metadata.run.clone(),
                valid: first.metadata.valid.clone(),
            },
            grid: first.grid.clone(),
            level_axis: LevelAxis {
                kind: LevelMetadata {
                    code: first.metadata.level.code,
                    description: "level_axis".to_string(),
                    value: None,
                    units: first.metadata.level.units.clone(),
                },
                levels,
            },
            values,
        });
    }

    Ok(stacked)
}

fn compare_level_metadata(left: &LevelMetadata, right: &LevelMetadata) -> Ordering {
    match (left.value, right.value) {
        (Some(left_value), Some(right_value)) => right_value
            .partial_cmp(&left_value)
            .unwrap_or(Ordering::Equal),
        _ => left.description.cmp(&right.description),
    }
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

fn forecast_duration(forecast_time: u32, unit: u8) -> Result<Duration> {
    match unit {
        0 => Ok(Duration::minutes(i64::from(forecast_time))),
        1 => Ok(Duration::hours(i64::from(forecast_time))),
        2 => Ok(Duration::days(i64::from(forecast_time))),
        _ => bail!("unsupported GRIB time range unit {}", unit),
    }
}

fn forecast_hour_from_duration(duration: Duration, original_unit: u8) -> Result<u16> {
    let minutes = duration.num_minutes();
    if minutes % 60 != 0 {
        bail!(
            "GRIB time range unit {} produced a non-hour-aligned forecast duration of {} minutes",
            original_unit,
            minutes
        );
    }

    u16::try_from(minutes / 60).context("forecast duration overflowed rustbox forecast-hour range")
}

fn validate_plan_matches_message(
    plan: &SubsetPlan,
    selection: &SubsetMessageRef,
    message_reference_time: DateTime<Utc>,
    message_forecast_hour: u16,
    message_valid_time: DateTime<Utc>,
) -> Result<()> {
    if plan.run.cycle != message_reference_time {
        bail!(
            "subset plan cycle {} does not match decoded message reference time {} for {} {}",
            plan.run.cycle,
            message_reference_time,
            selection.variable,
            selection.level
        );
    }

    if plan.run.forecast_hour != message_forecast_hour {
        bail!(
            "subset plan forecast hour {} does not match decoded message forecast hour {} for {} {}",
            plan.run.forecast_hour,
            message_forecast_hour,
            selection.variable,
            selection.level
        );
    }

    let plan_valid_time = plan.run.cycle + Duration::hours(i64::from(plan.run.forecast_hour));
    if plan_valid_time != message_valid_time {
        bail!(
            "subset plan valid time {} does not match decoded message valid time {} for {} {}",
            plan_valid_time,
            message_valid_time,
            selection.variable,
            selection.level
        );
    }

    Ok(())
}

fn validate_selection_matches_message(
    selection: &SubsetMessageRef,
    message: &grib_core::grib2::Grib2Message,
) -> Result<()> {
    let decoded_variable = decoded_short_name(
        message.discipline,
        message.product.parameter_category,
        message.product.parameter_number,
    )
    .context("decoded GRIB message did not map to a supported rustbox short name")?;
    if !decoded_variable.eq_ignore_ascii_case(selection.variable.trim()) {
        bail!(
            "subset selection expected variable {} but decoded GRIB message is {}",
            selection.variable,
            decoded_variable
        );
    }
    if !selection_level_matches_message(
        &selection.level,
        message.product.level_type,
        message.product.level_value,
    ) {
        bail!(
            "subset selection expected level {} but decoded GRIB message is {}",
            selection.level,
            canonical_level_description(
                message.product.level_type,
                message.product.level_value,
                level_name(message.product.level_type)
            )
        );
    }

    Ok(())
}

fn decoded_short_name(discipline: u8, category: u8, number: u8) -> Option<&'static str> {
    match (discipline, category, number) {
        (0, 0, 0) => Some("TMP"),
        (0, 0, 6) => Some("DPT"),
        (0, 1, 1) => Some("RH"),
        (0, 2, 2) => Some("UGRD"),
        (0, 2, 3) => Some("VGRD"),
        (0, 2, 22) => Some("GUST"),
        (0, 3, 0) => Some("PRES"),
        (0, 3, 5) => Some("HGT"),
        _ => None,
    }
}

fn selection_level_matches_message(selection_level: &str, level_type: u8, raw_value: f64) -> bool {
    let normalized = selection_level.trim().to_ascii_lowercase();
    if normalized == "surface" {
        return level_type == 1;
    }
    if let Some(hpa) = parse_level_value(&normalized, "mb") {
        return level_type == 100
            && (canonical_level_value(level_type, raw_value) - hpa).abs() < 1e-6;
    }
    if let Some(meters) = parse_level_value(&normalized, "m above ground") {
        return level_type == 103
            && (canonical_level_value(level_type, raw_value) - meters).abs() < 1e-6;
    }

    canonical_level_description(level_type, raw_value, level_name(level_type))
        .eq_ignore_ascii_case(selection_level.trim())
}

fn parse_level_value(level: &str, suffix: &str) -> Option<f64> {
    let value = level.strip_suffix(suffix)?.trim();
    value.parse::<f64>().ok()
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

fn canonical_level_description(level_type: u8, raw_value: f64, default_name: &str) -> String {
    match level_type {
        1 => "surface".to_string(),
        100 => format!("{:.0} mb", canonical_level_value(level_type, raw_value)),
        103 => format!(
            "{:.0} m above ground",
            canonical_level_value(level_type, raw_value)
        ),
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
    use serde::Deserialize;
    use std::path::PathBuf;
    use wx_fetch::{
        HrrrSelectionRequest, HrrrSubsetRequest, plan_hrrr_fixture_subset, plan_hrrr_subset,
    };
    use wx_severe::compute_significant_tornado_parameter;
    use wx_thermo::compute_parcel_diagnostics;
    use wx_types::ProjectionKind;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[derive(Debug, Deserialize)]
    struct ExpectedDiagnostics {
        sbcape_jkg: f64,
        sbcin_jkg: f64,
        mlcape_jkg: f64,
        mlcin_jkg: f64,
        mucape_jkg: f64,
        mucin_jkg: f64,
        stp_fixed: f64,
        srh_01km_m2s2: f64,
        srh_03km_m2s2: f64,
        bulk_shear_06km_ms: f64,
    }

    #[derive(Debug, Deserialize)]
    struct ModelColumnFixture {
        profile: SoundingProfile,
        expected: ExpectedDiagnostics,
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
                    forecast: None,
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
        let plan = plan_hrrr_fixture_subset(
            &HrrrSubsetRequest {
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
                        variable: "TMP".to_string(),
                        level: "2 m above ground".to_string(),
                        forecast: Some("anl".to_string()),
                    },
                ],
            },
            &idx_text,
            fragment_len,
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
    fn decoded_pressure_bundle_builds_stacked_3d_fields() {
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let pressure_plan = plan_hrrr_fixture_subset(
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
                                forecast: Some("anl".to_string()),
                            },
                            HrrrSelectionRequest {
                                variable: "TMP".to_string(),
                                level: level.to_string(),
                                forecast: Some("anl".to_string()),
                            },
                            HrrrSelectionRequest {
                                variable: "UGRD".to_string(),
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

        let bundle = decode_field_bundle(
            &fixture_path("hrrr_demo_pressure_fragment.grib2"),
            &pressure_plan,
        )
        .expect("bundle decode should succeed");

        assert_eq!(bundle.fields_2d.len(), pressure_plan.selections.len());
        let tmp_stack = bundle
            .fields_3d
            .iter()
            .find(|field| field.metadata.short_name == "TMP")
            .expect("TMP stack should exist");
        assert_eq!(
            tmp_stack.level_axis.levels.len(),
            REQUIRED_PRESSURE_LEVELS.len()
        );
        assert_eq!(tmp_stack.expected_len(), tmp_stack.values.len());
    }

    #[test]
    fn decoded_bundle_summary_reports_3d_stacks() {
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let pressure_plan = plan_hrrr_fixture_subset(
            &HrrrSubsetRequest {
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
                        variable: "TMP".to_string(),
                        level: "700 mb".to_string(),
                        forecast: Some("anl".to_string()),
                    },
                ],
            },
            &std::fs::read_to_string(fixture_path("hrrr_demo_pressure_fragment.idx"))
                .expect("pressure idx should be readable"),
            std::fs::metadata(fixture_path("hrrr_demo_pressure_fragment.grib2"))
                .expect("pressure fragment should exist")
                .len(),
        )
        .expect("pressure plan should succeed");

        let bundle = decode_field_bundle(
            &fixture_path("hrrr_demo_pressure_fragment.grib2"),
            &pressure_plan,
        )
        .expect("bundle decode should succeed");
        let summary = summarize_field_bundle(&bundle);

        assert_eq!(summary.fields_2d.len(), 2);
        assert_eq!(summary.fields_3d.len(), 1);
        assert_eq!(summary.fields_3d[0].level_count, 2);
        assert!(
            summary.fields_3d[0]
                .levels
                .iter()
                .any(|level| level == "850 mb")
        );
    }

    #[test]
    fn decode_fragment_rejects_ranges_with_multiple_messages() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_gust_surface_fragment.idx"))
            .expect("fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let mut plan = plan_hrrr_subset(
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
        plan.selections[0].end_exclusive =
            std::fs::metadata(fixture_path("hrrr_gust_surface_fragment.grib2"))
                .expect("fixture should exist")
                .len();

        let error =
            decode_selected_message(&fixture_path("hrrr_gust_surface_fragment.grib2"), &plan)
                .expect_err("over-wide selection should fail");

        assert!(
            error
                .to_string()
                .contains("expected exactly one GRIB2 message"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn decode_fragment_rejects_cycle_mismatch_between_plan_and_message() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_gust_surface_fragment.idx"))
            .expect("fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let mut plan = plan_hrrr_subset(
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
        plan.run.cycle = cycle + Duration::hours(1);

        let error =
            decode_selected_message(&fixture_path("hrrr_gust_surface_fragment.grib2"), &plan)
                .expect_err("cycle mismatch should fail");

        assert!(
            error
                .to_string()
                .contains("does not match decoded message reference time"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn decode_fragment_rejects_forecast_hour_mismatch_between_plan_and_message() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_gust_surface_fragment.idx"))
            .expect("fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let mut plan = plan_hrrr_subset(
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
        plan.run.forecast_hour = 1;

        let error =
            decode_selected_message(&fixture_path("hrrr_gust_surface_fragment.grib2"), &plan)
                .expect_err("forecast mismatch should fail");

        assert!(
            error
                .to_string()
                .contains("does not match decoded message forecast hour"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn decode_fragment_rejects_variable_identity_mismatches() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_gust_surface_fragment.idx"))
            .expect("fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let mut plan = plan_hrrr_subset(
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
        plan.selections[0].variable = "PRES".to_string();

        let error =
            decode_selected_message(&fixture_path("hrrr_gust_surface_fragment.grib2"), &plan)
                .expect_err("variable mismatch should fail");

        assert!(
            error
                .to_string()
                .contains("expected variable PRES but decoded GRIB message is GUST"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn decode_fragment_rejects_level_identity_mismatches() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_gust_surface_fragment.idx"))
            .expect("fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let mut plan = plan_hrrr_subset(
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
        plan.selections[0].level = "250 mb".to_string();

        let error =
            decode_selected_message(&fixture_path("hrrr_gust_surface_fragment.grib2"), &plan)
                .expect_err("level mismatch should fail");

        assert!(
            error
                .to_string()
                .contains("expected level 250 mb but decoded GRIB message is surface"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn decode_fragment_rejects_wrong_message_identity_even_when_time_matches() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_gust_surface_fragment.idx"))
            .expect("fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let mut plan = plan_hrrr_subset(
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

        let second_message_start = 1_136_310;
        let second_message_end =
            std::fs::metadata(fixture_path("hrrr_gust_surface_fragment.grib2"))
                .expect("fixture should exist")
                .len();
        plan.selections[0].start = second_message_start;
        plan.selections[0].end_exclusive = second_message_end;

        let error =
            decode_selected_message(&fixture_path("hrrr_gust_surface_fragment.grib2"), &plan)
                .expect_err("wrong-message identity should fail");

        assert!(
            error
                .to_string()
                .contains("expected variable GUST but decoded GRIB message is UGRD"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn extracted_hrrr_column_builds_valid_model_profile() {
        let model_fixture: ModelColumnFixture = serde_json::from_str(
            &std::fs::read_to_string(fixture_path("hrrr_demo_model_column.json"))
                .expect("model column fixture should be readable"),
        )
        .expect("model column fixture should parse");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");

        let surface_plan = plan_hrrr_fixture_subset(
            &HrrrSubsetRequest {
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
            },
            &std::fs::read_to_string(fixture_path("hrrr_demo_surface_fragment.idx"))
                .expect("surface idx should be readable"),
            std::fs::metadata(fixture_path("hrrr_demo_surface_fragment.grib2"))
                .expect("surface fragment should exist")
                .len(),
        )
        .expect("surface plan should succeed");
        let pressure_plan = plan_hrrr_fixture_subset(
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

        let (x, y) = find_valid_hrrr_profile_point(&surface_messages, &pressure_messages)
            .expect("valid point");
        assert_eq!(x, model_fixture.profile.grid_x.expect("fixture grid_x"));
        assert_eq!(y, model_fixture.profile.grid_y.expect("fixture grid_y"));
        let profile = build_hrrr_sounding_profile(&surface_messages, &pressure_messages, x, y)
            .expect("profile extraction should succeed");

        assert!(profile.levels.len() >= 8);
        assert!(profile.valid_time.is_some());
        assert_eq!(profile.grid_x, Some(x));
        assert_eq!(profile.grid_y, Some(y));
        assert_eq!(profile.station_id, model_fixture.profile.station_id);
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
        assert_eq!(profile.levels.len(), model_fixture.profile.levels.len());
        for (actual, expected) in profile
            .levels
            .iter()
            .zip(model_fixture.profile.levels.iter())
        {
            assert!((actual.pressure_hpa - expected.pressure_hpa).abs() < 0.01);
            assert!((actual.height_m - expected.height_m).abs() < 0.01);
            assert!((actual.temperature_c - expected.temperature_c).abs() < 0.01);
            assert!((actual.dewpoint_c - expected.dewpoint_c).abs() < 0.01);
            assert!((actual.wind_direction_deg - expected.wind_direction_deg).abs() < 0.01);
            assert!((actual.wind_speed_kts - expected.wind_speed_kts).abs() < 0.01);
        }

        let parcel = compute_parcel_diagnostics(&profile).expect("parcel diagnostics should work");
        assert!((parcel.surface.cape_jkg - model_fixture.expected.sbcape_jkg).abs() < 0.01);
        assert!((parcel.surface.cin_jkg - model_fixture.expected.sbcin_jkg).abs() < 0.01);
        assert!((parcel.mixed_layer.cape_jkg - model_fixture.expected.mlcape_jkg).abs() < 0.01);
        assert!((parcel.mixed_layer.cin_jkg - model_fixture.expected.mlcin_jkg).abs() < 0.01);
        assert!((parcel.most_unstable.cape_jkg - model_fixture.expected.mucape_jkg).abs() < 0.01);
        assert!((parcel.most_unstable.cin_jkg - model_fixture.expected.mucin_jkg).abs() < 0.01);

        let severe = compute_significant_tornado_parameter(&profile, &parcel)
            .expect("severe diagnostics should work");
        assert!(
            (severe.significant_tornado_parameter - model_fixture.expected.stp_fixed).abs() < 0.01
        );
        assert!(
            (severe.kinematics.srh_01km_m2s2 - model_fixture.expected.srh_01km_m2s2).abs() < 0.01
        );
        assert!(
            (severe.kinematics.srh_03km_m2s2 - model_fixture.expected.srh_03km_m2s2).abs() < 0.01
        );
        assert!(
            (severe.kinematics.bulk_shear_06km_ms - model_fixture.expected.bulk_shear_06km_ms)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn forecast_duration_rejects_unsupported_grib_time_units() {
        let error = forecast_duration(1, 13).expect_err("unsupported unit should fail");

        assert!(
            error
                .to_string()
                .contains("unsupported GRIB time range unit"),
            "unexpected error: {error}"
        );
    }
}
