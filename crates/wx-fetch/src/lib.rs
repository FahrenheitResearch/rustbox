pub mod cache;
pub mod client;
pub mod sources;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wx_types::{
    ArchiveCycleRecord, ArchiveCycleState, ArchiveJobSpec, RequestedField, RunMetadata,
    SourceMetadata,
};

pub use cache::DiskCache;
pub use client::{DownloadClient, DownloadConfig};
pub use sources::{HrrrSourceCandidate, hrrr_source_candidates};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HrrrSelectionRequest {
    pub variable: String,
    pub level: String,
    pub forecast: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HrrrSubsetRequest {
    pub cycle: DateTime<Utc>,
    pub forecast_hour: u16,
    pub product: String,
    pub selections: Vec<HrrrSelectionRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdxEntry {
    pub message_number: u32,
    pub byte_offset: u64,
    pub reference_time: DateTime<Utc>,
    pub variable: String,
    pub level: String,
    pub forecast: String,
    pub time_range: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsetMessageRef {
    pub message_number: u32,
    pub start: u64,
    pub end_exclusive: u64,
    pub variable: String,
    pub level: String,
    pub forecast: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ByteRangeOrigin {
    SourceObject,
    FixtureFragment { fragment_length: u64 },
    StagedSubset { fragment_length: u64 },
}

impl ByteRangeOrigin {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SourceObject => "source_object",
            Self::FixtureFragment { .. } => "fixture_fragment_rebased",
            Self::StagedSubset { .. } => "staged_subset_rebased",
        }
    }

    pub fn known_length(&self) -> Option<u64> {
        match self {
            Self::SourceObject => None,
            Self::FixtureFragment { fragment_length } | Self::StagedSubset { fragment_length } => {
                Some(*fragment_length)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubsetPlan {
    pub source: SourceMetadata,
    pub run: RunMetadata,
    pub source_name: String,
    pub source_grib_url: String,
    pub source_idx_url: String,
    pub byte_range_origin: ByteRangeOrigin,
    pub selections: Vec<SubsetMessageRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StagedSubset {
    pub source_plan: SubsetPlan,
    pub local_plan: SubsetPlan,
    pub local_grib_path: PathBuf,
    pub staged_bytes: u64,
}

impl From<&RequestedField> for HrrrSelectionRequest {
    fn from(value: &RequestedField) -> Self {
        Self {
            variable: value.variable.clone(),
            level: value.level.clone(),
            forecast: value.forecast.clone(),
        }
    }
}

pub fn parse_idx(text: &str) -> Result<Vec<IdxEntry>> {
    let mut entries = Vec::new();

    for (line_number, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(8, ':').collect();
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

        let forecast = parts[5].trim_end_matches(':').to_string();
        entries.push(IdxEntry {
            message_number,
            byte_offset,
            reference_time: Utc.from_utc_datetime(&naive_time),
            variable: parts[3].to_string(),
            level: parts[4].to_string(),
            time_range: parse_time_range(&forecast),
            forecast,
        });
    }

    if entries.is_empty() {
        bail!("idx manifest contained no entries");
    }

    Ok(entries)
}

pub fn iter_hrrr_cycles(
    start_cycle: DateTime<Utc>,
    end_cycle: DateTime<Utc>,
    cycle_step_hours: u16,
) -> Result<Vec<DateTime<Utc>>> {
    if end_cycle < start_cycle {
        bail!(
            "end cycle {} precedes start cycle {}",
            end_cycle,
            start_cycle
        );
    }
    if cycle_step_hours == 0 {
        bail!("cycle step hours must be at least 1");
    }

    let mut cycles = Vec::new();
    let mut current = start_cycle;
    let step = Duration::hours(i64::from(cycle_step_hours));
    while current <= end_cycle {
        cycles.push(current);
        current += step;
    }

    Ok(cycles)
}

pub fn build_hrrr_archive_requests(job: &ArchiveJobSpec) -> Result<Vec<HrrrSubsetRequest>> {
    if !job.model.eq_ignore_ascii_case("hrrr") {
        bail!(
            "rustbox archive-core currently supports HRRR job specs only, not {}",
            job.model
        );
    }

    let selections: Vec<HrrrSelectionRequest> = job
        .selections
        .iter()
        .map(HrrrSelectionRequest::from)
        .collect();
    iter_hrrr_cycles(job.start_cycle, job.end_cycle, job.cycle_step_hours)?
        .into_iter()
        .map(|cycle| {
            Ok(HrrrSubsetRequest {
                cycle,
                forecast_hour: job.forecast_hour,
                product: job.product.clone(),
                selections: selections.clone(),
            })
        })
        .collect()
}

pub fn probe_first_available_hrrr_source(
    client: &DownloadClient,
    request: &HrrrSubsetRequest,
) -> Result<HrrrSourceCandidate> {
    for candidate in hrrr_source_candidates(request.cycle, &request.product, request.forecast_hour)
    {
        if client.head_ok(&candidate.idx_url) || client.head_ok(&candidate.grib_url) {
            return Ok(candidate);
        }
    }

    bail!(
        "no reachable HRRR source candidates for {} {} f{:02}",
        request.cycle,
        request.product,
        request.forecast_hour
    )
}

pub fn fetch_remote_idx_text(
    client: &DownloadClient,
    candidate: &HrrrSourceCandidate,
) -> Result<String> {
    client
        .get_text(&candidate.idx_url)
        .with_context(|| format!("failed to fetch idx {}", candidate.idx_url))
}

pub fn plan_hrrr_subset(request: &HrrrSubsetRequest, idx_text: &str) -> Result<SubsetPlan> {
    let default_source =
        hrrr_source_candidates(request.cycle, &request.product, request.forecast_hour)
            .into_iter()
            .find(|candidate| candidate.name == "aws")
            .context("default AWS HRRR source template should exist")?;
    plan_hrrr_subset_with_context(
        request,
        idx_text,
        ByteRangeOrigin::SourceObject,
        &default_source,
    )
}

pub fn plan_hrrr_remote_subset(
    request: &HrrrSubsetRequest,
    idx_text: &str,
    candidate: &HrrrSourceCandidate,
) -> Result<SubsetPlan> {
    plan_hrrr_subset_with_context(request, idx_text, ByteRangeOrigin::SourceObject, candidate)
}

pub fn plan_hrrr_fixture_subset(
    request: &HrrrSubsetRequest,
    idx_text: &str,
    fragment_length: u64,
) -> Result<SubsetPlan> {
    let fixture_source = HrrrSourceCandidate {
        name: "fixture".to_string(),
        priority: 0,
        grib_url: format!(
            "https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.{}/conus/hrrr.t{}z.{}f{:02}.grib2",
            request.cycle.format("%Y%m%d"),
            request.cycle.format("%H"),
            normalized_hrrr_product(&request.product),
            request.forecast_hour
        ),
        idx_url: format!(
            "https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.{}/conus/hrrr.t{}z.{}f{:02}.grib2.idx",
            request.cycle.format("%Y%m%d"),
            request.cycle.format("%H"),
            normalized_hrrr_product(&request.product),
            request.forecast_hour
        ),
    };
    plan_hrrr_subset_with_context(
        request,
        idx_text,
        ByteRangeOrigin::FixtureFragment { fragment_length },
        &fixture_source,
    )
}

pub fn stage_remote_subset_download(
    client: &DownloadClient,
    plan: &SubsetPlan,
    output_path: &Path,
) -> Result<StagedSubset> {
    if plan.selections.is_empty() {
        bail!("subset plan contained no selected messages to stage");
    }
    std::fs::create_dir_all(
        output_path
            .parent()
            .context("staged subset output path must have a parent directory")?,
    )?;

    let ranges: Vec<(u64, u64)> = plan
        .selections
        .iter()
        .map(|selection| (selection.start, selection.end_exclusive))
        .collect();
    let bytes = client
        .get_ranges(&plan.source_grib_url, &ranges)
        .with_context(|| {
            format!(
                "failed to download staged ranges from {}",
                plan.source_grib_url
            )
        })?;
    std::fs::write(output_path, &bytes)
        .with_context(|| format!("failed to write staged subset {}", output_path.display()))?;

    let local_plan = rebase_plan_for_staged_subset(plan, &bytes)?;
    Ok(StagedSubset {
        source_plan: plan.clone(),
        local_plan,
        local_grib_path: output_path.to_path_buf(),
        staged_bytes: bytes.len() as u64,
    })
}

pub fn stage_subset_file_name(plan: &SubsetPlan) -> String {
    format!(
        "{}_{}_f{:02}_{}.grib2",
        plan.source.model,
        plan.run.cycle.format("%Y%m%d%H"),
        plan.run.forecast_hour,
        plan.source.product
    )
}

pub fn initial_archive_manifest(job: &ArchiveJobSpec) -> Result<wx_types::ArchiveRunManifest> {
    let cycles = iter_hrrr_cycles(job.start_cycle, job.end_cycle, job.cycle_step_hours)?
        .into_iter()
        .map(|cycle| ArchiveCycleRecord {
            cycle,
            source_name: None,
            source_grib_url: None,
            source_idx_url: None,
            subset_path: None,
            staged_manifest_path: None,
            decoded_summary_path: None,
            persisted_store_path: None,
            state: ArchiveCycleState::Planned,
            message_count: 0,
            field_count_2d: 0,
            field_count_3d: 0,
            error: None,
        })
        .collect();

    Ok(wx_types::ArchiveRunManifest {
        job: job.clone(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        cycles,
    })
}

fn plan_hrrr_subset_with_context(
    request: &HrrrSubsetRequest,
    idx_text: &str,
    byte_range_origin: ByteRangeOrigin,
    source_candidate: &HrrrSourceCandidate,
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
        if let Some(entry_forecast_hour) = forecast_hour_from_idx_entry(entry)
            && entry_forecast_hour != request.forecast_hour
        {
            bail!(
                "idx entry {} forecast semantics resolve to f{:02}, not requested f{:02}",
                selector_label(wanted),
                entry_forecast_hour,
                request.forecast_hour
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

    Ok(SubsetPlan {
        source: SourceMetadata {
            provider: format!("hrrr-{}", source_candidate.name),
            model: "hrrr".to_string(),
            product: request.product.clone(),
        },
        run: RunMetadata {
            cycle: request.cycle,
            forecast_hour: request.forecast_hour,
        },
        source_name: source_candidate.name.clone(),
        source_grib_url: source_candidate.grib_url.clone(),
        source_idx_url: source_candidate.idx_url.clone(),
        byte_range_origin,
        selections,
    })
}

fn rebase_plan_for_staged_subset(plan: &SubsetPlan, bytes: &[u8]) -> Result<SubsetPlan> {
    let mut rebased_start = 0u64;
    let mut local_selections = Vec::with_capacity(plan.selections.len());

    for selection in &plan.selections {
        let expected_len = selection
            .end_exclusive
            .checked_sub(selection.start)
            .context("selection byte range underflowed during staging")?;
        let rebased_end = rebased_start
            .checked_add(expected_len)
            .context("rebased staged selection overflowed")?;
        local_selections.push(SubsetMessageRef {
            message_number: selection.message_number,
            start: rebased_start,
            end_exclusive: rebased_end,
            variable: selection.variable.clone(),
            level: selection.level.clone(),
            forecast: selection.forecast.clone(),
        });
        rebased_start = rebased_end;
    }

    if rebased_start != bytes.len() as u64 {
        bail!(
            "rebased staged subset length {} does not match downloaded byte length {}",
            rebased_start,
            bytes.len()
        );
    }

    let mut local_plan = plan.clone();
    local_plan.byte_range_origin = ByteRangeOrigin::StagedSubset {
        fragment_length: bytes.len() as u64,
    };
    local_plan.selections = local_selections;
    Ok(local_plan)
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

fn forecast_hour_from_idx_entry(entry: &IdxEntry) -> Option<u16> {
    if let Some((_, end_hour)) = entry.time_range {
        return u16::try_from(end_hour).ok();
    }

    forecast_hour_from_idx_forecast(&entry.forecast)
}

fn forecast_hour_from_idx_forecast(forecast: &str) -> Option<u16> {
    let trimmed = forecast.trim();
    let normalized = trimmed.to_ascii_lowercase();

    if matches!(normalized.as_str(), "anl" | "analysis" | "2dvaranl") {
        return Some(0);
    }
    if let Some((_, end_hour)) = parse_time_range(&normalized) {
        return u16::try_from(end_hour).ok();
    }

    parse_single_hour(&normalized).and_then(|hour| u16::try_from(hour).ok())
}

fn parse_time_range(forecast: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = forecast.split_whitespace().collect();
    for (index, part) in parts.iter().enumerate() {
        if part.contains('-') && index + 1 < parts.len() && parts[index + 1].starts_with("hr") {
            let range_parts: Vec<&str> = part.split('-').collect();
            if range_parts.len() == 2
                && let (Ok(start), Ok(end)) =
                    (range_parts[0].parse::<u32>(), range_parts[1].parse::<u32>())
            {
                return Some((start, end));
            }
        }
    }

    None
}

fn parse_single_hour(forecast: &str) -> Option<u32> {
    let parts: Vec<&str> = forecast.split_whitespace().collect();
    if parts.len() < 3 || parts[1] != "hr" || !parts[2].ends_with("fcst") {
        return None;
    }

    parts[0].parse::<u32>().ok()
}

fn normalized_hrrr_product(product: &str) -> &'static str {
    match product.trim().to_ascii_lowercase().as_str() {
        "sfc" | "surface" | "wrfsfc" => "wrfsfc",
        "prs" | "pressure" | "wrfprs" => "wrfprs",
        "nat" | "native" | "wrfnat" => "wrfnat",
        "subh" | "subhourly" | "wrfsubh" => "wrfsubh",
        _ => "wrfsfc",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
        assert_eq!(plan.source_name, "aws");
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

    #[test]
    fn plan_hrrr_subset_rejects_forecast_hour_semantic_mismatches() {
        let idx_text = "1:0:d=2024040100:TMP:500 mb:1 hr fcst:";
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");

        let error = plan_hrrr_subset(
            &HrrrSubsetRequest {
                cycle,
                forecast_hour: 0,
                product: "prs".to_string(),
                selections: vec![HrrrSelectionRequest {
                    variable: "TMP".to_string(),
                    level: "500 mb".to_string(),
                    forecast: Some("1 hr fcst".to_string()),
                }],
            },
            idx_text,
        )
        .expect_err("forecast hour mismatch should fail");

        assert!(
            error
                .to_string()
                .contains("forecast semantics resolve to f01"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn iter_hrrr_cycles_is_inclusive() {
        let start = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let end = Utc
            .with_ymd_and_hms(2024, 4, 1, 3, 0, 0)
            .single()
            .expect("valid cycle");

        let cycles = iter_hrrr_cycles(start, end, 1).expect("cycle iteration should work");
        assert_eq!(cycles.len(), 4);
        assert_eq!(cycles[0], start);
        assert_eq!(cycles[3], end);
    }

    #[test]
    fn build_archive_requests_expands_every_cycle() {
        let job = ArchiveJobSpec {
            model: "hrrr".to_string(),
            product: "prs".to_string(),
            start_cycle: Utc
                .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
                .single()
                .expect("valid cycle"),
            end_cycle: Utc
                .with_ymd_and_hms(2024, 4, 1, 2, 0, 0)
                .single()
                .expect("valid cycle"),
            cycle_step_hours: 1,
            forecast_hour: 0,
            selections: vec![RequestedField {
                variable: "TMP".to_string(),
                level: "850 mb".to_string(),
                forecast: Some("anl".to_string()),
            }],
            output_root: "target/archive".to_string(),
        };

        let requests = build_hrrr_archive_requests(&job).expect("job expansion should work");
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].product, "prs");
        assert_eq!(requests[1].cycle.format("%H").to_string(), "01");
    }

    #[test]
    fn staged_subset_rebases_offsets_contiguously() {
        let plan = SubsetPlan {
            source: SourceMetadata {
                provider: "test".to_string(),
                model: "hrrr".to_string(),
                product: "prs".to_string(),
            },
            run: RunMetadata {
                cycle: Utc
                    .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
                    .single()
                    .expect("valid cycle"),
                forecast_hour: 0,
            },
            source_name: "aws".to_string(),
            source_grib_url: "https://example.com/test.grib2".to_string(),
            source_idx_url: "https://example.com/test.grib2.idx".to_string(),
            byte_range_origin: ByteRangeOrigin::SourceObject,
            selections: vec![
                SubsetMessageRef {
                    message_number: 1,
                    start: 100,
                    end_exclusive: 110,
                    variable: "TMP".to_string(),
                    level: "850 mb".to_string(),
                    forecast: "anl".to_string(),
                },
                SubsetMessageRef {
                    message_number: 2,
                    start: 120,
                    end_exclusive: 140,
                    variable: "UGRD".to_string(),
                    level: "850 mb".to_string(),
                    forecast: "anl".to_string(),
                },
            ],
        };

        let local_plan = rebase_plan_for_staged_subset(&plan, &[0_u8; 30]).expect("rebase works");
        assert_eq!(
            local_plan.byte_range_origin,
            ByteRangeOrigin::StagedSubset {
                fragment_length: 30
            }
        );
        assert_eq!(local_plan.selections[0].start, 0);
        assert_eq!(local_plan.selections[0].end_exclusive, 10);
        assert_eq!(local_plan.selections[1].start, 10);
        assert_eq!(local_plan.selections[1].end_exclusive, 30);
    }
}
