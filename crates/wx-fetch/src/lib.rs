use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use wx_types::{RunMetadata, SourceMetadata};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HrrrSelectionRequest {
    pub variable: String,
    pub level: String,
    pub forecast: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HrrrSubsetRequest {
    pub cycle: DateTime<Utc>,
    pub forecast_hour: u16,
    pub product: String,
    pub selections: Vec<HrrrSelectionRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdxEntry {
    pub message_number: u32,
    pub byte_offset: u64,
    pub reference_time: DateTime<Utc>,
    pub variable: String,
    pub level: String,
    pub forecast: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubsetMessageRef {
    pub message_number: u32,
    pub start: u64,
    pub end_exclusive: u64,
    pub variable: String,
    pub level: String,
    pub forecast: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ByteRangeOrigin {
    SourceObject,
    FixtureFragment { fragment_length: u64 },
}

impl ByteRangeOrigin {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SourceObject => "source_object",
            Self::FixtureFragment { .. } => "fixture_fragment_rebased",
        }
    }

    pub fn known_length(&self) -> Option<u64> {
        match self {
            Self::SourceObject => None,
            Self::FixtureFragment { fragment_length } => Some(*fragment_length),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubsetPlan {
    pub source: SourceMetadata,
    pub run: RunMetadata,
    pub source_grib_url: String,
    pub source_idx_url: String,
    pub byte_range_origin: ByteRangeOrigin,
    pub selections: Vec<SubsetMessageRef>,
}

pub fn parse_idx(text: &str) -> Result<Vec<IdxEntry>> {
    let mut entries = Vec::new();

    for (line_number, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 6 {
            bail!("idx line {} is malformed: {}", line_number + 1, line);
        }

        let message_number = parts[0]
            .parse::<u32>()
            .with_context(|| format!("invalid message number on idx line {}", line_number + 1))?;
        let byte_offset = parts[1]
            .parse::<u64>()
            .with_context(|| format!("invalid byte offset on idx line {}", line_number + 1))?;
        let timestamp = parts[2]
            .strip_prefix("d=")
            .ok_or_else(|| anyhow!("missing d= timestamp on idx line {}", line_number + 1))?;
        let timestamp_with_minutes = format!("{timestamp}00");
        let naive_time = NaiveDateTime::parse_from_str(&timestamp_with_minutes, "%Y%m%d%H%M")
            .with_context(|| format!("invalid timestamp on idx line {}", line_number + 1))?;

        entries.push(IdxEntry {
            message_number,
            byte_offset,
            reference_time: Utc.from_utc_datetime(&naive_time),
            variable: parts[3].to_string(),
            level: parts[4].to_string(),
            forecast: parts[5].trim_end_matches(':').to_string(),
        });
    }

    if entries.is_empty() {
        bail!("idx manifest contained no entries");
    }

    Ok(entries)
}

pub fn plan_hrrr_subset(request: &HrrrSubsetRequest, idx_text: &str) -> Result<SubsetPlan> {
    plan_hrrr_subset_with_context(request, idx_text, ByteRangeOrigin::SourceObject)
}

pub fn plan_hrrr_fixture_subset(
    request: &HrrrSubsetRequest,
    idx_text: &str,
    fragment_length: u64,
) -> Result<SubsetPlan> {
    plan_hrrr_subset_with_context(
        request,
        idx_text,
        ByteRangeOrigin::FixtureFragment { fragment_length },
    )
}

fn plan_hrrr_subset_with_context(
    request: &HrrrSubsetRequest,
    idx_text: &str,
    byte_range_origin: ByteRangeOrigin,
) -> Result<SubsetPlan> {
    let entries = parse_idx(idx_text)?;
    let mut selections = Vec::new();

    if request.selections.is_empty() {
        bail!("subset request must include at least one variable/level selection");
    }

    for wanted in &request.selections {
        let matches: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                matches_selector(&entry.variable, &wanted.variable)
                    && matches_selector(&entry.level, &wanted.level)
                    && wanted
                        .forecast
                        .as_ref()
                        .is_none_or(|forecast| matches_selector(&entry.forecast, forecast))
            })
            .collect();

        let (entry_index, entry) = match matches.as_slice() {
            [] => Err(anyhow!("no idx entries matched {}", selector_label(wanted))),
            [single] => Ok(*single),
            _ => Err(anyhow!(
                "selector {} matched multiple idx entries; specify forecast to disambiguate",
                selector_label(wanted)
            )),
        }?;

        if entry.reference_time != request.cycle {
            bail!(
                "idx entry {} reference time {} does not match requested cycle {}",
                selector_label(wanted),
                entry.reference_time,
                request.cycle
            );
        }

        let end_exclusive = entries
            .get(entry_index + 1)
            .map(|next| next.byte_offset)
            .or_else(|| byte_range_origin.known_length())
            .ok_or_else(|| {
                anyhow!(
                    "selected idx entry {} has no following offset and no known object length",
                    selector_label(wanted)
                )
            })?;

        if end_exclusive <= entry.byte_offset {
            bail!(
                "invalid byte range for {}:{} ({}..{})",
                wanted.variable,
                wanted.level,
                entry.byte_offset,
                end_exclusive
            );
        }

        selections.push(SubsetMessageRef {
            message_number: entry.message_number,
            start: entry.byte_offset,
            end_exclusive,
            variable: entry.variable.clone(),
            level: entry.level.clone(),
            forecast: entry.forecast.clone(),
        });
    }

    let cycle_date = request.cycle.format("%Y%m%d").to_string();
    let cycle_hour = request.cycle.format("%H").to_string();
    let source_grib_url = format!(
        "https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.{}/conus/hrrr.t{}z.wrf{}f{:02}.grib2",
        cycle_date, cycle_hour, request.product, request.forecast_hour
    );
    let source_idx_url = format!("{}.idx", source_grib_url);

    Ok(SubsetPlan {
        source: SourceMetadata {
            provider: "noaa-hrrr-bdp-pds".to_string(),
            model: "hrrr".to_string(),
            product: request.product.clone(),
        },
        run: RunMetadata {
            cycle: request.cycle,
            forecast_hour: request.forecast_hour,
        },
        source_grib_url,
        source_idx_url,
        byte_range_origin,
        selections,
    })
}

fn matches_selector(candidate: &str, wanted: &str) -> bool {
    candidate.trim().eq_ignore_ascii_case(wanted.trim())
}

fn selector_label(request: &HrrrSelectionRequest) -> String {
    match request.forecast.as_deref() {
        Some(forecast) => format!("{}:{}:{}", request.variable, request.level, forecast),
        None => format!("{}:{}", request.variable, request.level),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[test]
    fn plan_hrrr_subset_selects_expected_message() {
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

        assert_eq!(plan.source.model, "hrrr");
        assert_eq!(plan.source.product, "sfc");
        assert_eq!(
            plan.source_grib_url,
            "https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.20240401/conus/hrrr.t00z.wrfsfcf00.grib2"
        );
        assert_eq!(plan.source_idx_url, format!("{}.idx", plan.source_grib_url));
        assert_eq!(plan.byte_range_origin, ByteRangeOrigin::SourceObject);
        assert_eq!(plan.selections.len(), 1);

        let selection = &plan.selections[0];
        assert_eq!(selection.message_number, 9);
        assert_eq!(selection.start, 0);
        assert_eq!(selection.end_exclusive, 1_136_310);
        assert_eq!(selection.variable, "GUST");
        assert_eq!(selection.level, "surface");
        assert_eq!(selection.forecast, "anl");
    }

    #[test]
    fn plan_hrrr_subset_supports_multiple_requested_messages() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_demo_surface_fragment.idx"))
            .expect("surface fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let known_length = std::fs::metadata(fixture_path("hrrr_demo_surface_fragment.grib2"))
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
                        variable: "TMP".to_string(),
                        level: "2 m above ground".to_string(),
                        forecast: Some("anl".to_string()),
                    },
                    HrrrSelectionRequest {
                        variable: "VGRD".to_string(),
                        level: "10 m above ground".to_string(),
                        forecast: Some("anl".to_string()),
                    },
                ],
            },
            &idx_text,
            known_length,
        )
        .expect("plan should succeed");

        assert_eq!(plan.selections.len(), 3);
        assert_eq!(
            plan.byte_range_origin,
            ByteRangeOrigin::FixtureFragment {
                fragment_length: known_length
            }
        );
        assert_eq!(plan.selections[0].variable, "GUST");
        assert_eq!(plan.selections[1].level, "2 m above ground");
        assert_eq!(plan.selections[2].variable, "VGRD");
        assert_eq!(plan.selections[2].end_exclusive, known_length);
    }

    #[test]
    fn duplicate_var_level_without_forecast_is_an_error() {
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let idx_text = "\
1:0:d=2024040100:TMP:surface:anl:\n\
2:50:d=2024040100:TMP:surface:1 hour fcst:\n\
3:100:d=2024040100:GUST:surface:anl:\n";

        let error = plan_hrrr_subset(
            &HrrrSubsetRequest {
                cycle,
                forecast_hour: 0,
                product: "sfc".to_string(),
                selections: vec![HrrrSelectionRequest {
                    variable: "TMP".to_string(),
                    level: "surface".to_string(),
                    forecast: None,
                }],
            },
            idx_text,
        )
        .expect_err("ambiguous selector should fail");

        assert!(
            error.to_string().contains("matched multiple idx entries"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn plan_hrrr_subset_rejects_idx_cycle_mismatches() {
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_gust_surface_fragment.idx"))
            .expect("fixture idx should be readable");
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 2, 0, 0, 0)
            .single()
            .expect("valid cycle");

        let error = plan_hrrr_subset(
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
        .expect_err("cycle mismatch should fail");

        assert!(
            error.to_string().contains("does not match requested cycle"),
            "unexpected error: {error}"
        );
    }
}
