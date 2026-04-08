use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use wx_types::{RunMetadata, SourceMetadata};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HrrrSubsetRequest {
    pub cycle: DateTime<Utc>,
    pub forecast_hour: u16,
    pub product: String,
    pub variable: String,
    pub level: String,
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

#[derive(Debug, Clone, PartialEq)]
pub struct SubsetPlan {
    pub source: SourceMetadata,
    pub run: RunMetadata,
    pub grib_url: String,
    pub idx_url: String,
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
    let entries = parse_idx(idx_text)?;
    let mut selections = Vec::new();

    for (index, entry) in entries.iter().enumerate() {
        if !entry.variable.eq_ignore_ascii_case(&request.variable) {
            continue;
        }
        if !entry
            .level
            .to_lowercase()
            .contains(&request.level.to_lowercase())
        {
            continue;
        }

        let next_offset = entries
            .get(index + 1)
            .map(|next| next.byte_offset)
            .ok_or_else(|| {
                anyhow!("selected idx entry has no following offset to bound the byte range")
            })?;

        selections.push(SubsetMessageRef {
            message_number: entry.message_number,
            start: entry.byte_offset,
            end_exclusive: next_offset,
            variable: entry.variable.clone(),
            level: entry.level.clone(),
            forecast: entry.forecast.clone(),
        });
    }

    if selections.is_empty() {
        bail!(
            "no idx entries matched {}:{}",
            request.variable,
            request.level
        );
    }

    let cycle_date = request.cycle.format("%Y%m%d").to_string();
    let cycle_hour = request.cycle.format("%H").to_string();
    let grib_url = format!(
        "https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.{}/conus/hrrr.t{}z.wrf{}f{:02}.grib2",
        cycle_date, cycle_hour, request.product, request.forecast_hour
    );
    let idx_url = format!("{}.idx", grib_url);

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
        grib_url,
        idx_url,
        selections,
    })
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
                variable: "GUST".to_string(),
                level: "surface".to_string(),
            },
            &idx_text,
        )
        .expect("plan should succeed");

        assert_eq!(plan.source.model, "hrrr");
        assert_eq!(plan.source.product, "sfc");
        assert_eq!(
            plan.grib_url,
            "https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.20240401/conus/hrrr.t00z.wrfsfcf00.grib2"
        );
        assert_eq!(plan.idx_url, format!("{}.idx", plan.grib_url));
        assert_eq!(plan.selections.len(), 1);

        let selection = &plan.selections[0];
        assert_eq!(selection.message_number, 9);
        assert_eq!(selection.start, 0);
        assert_eq!(selection.end_exclusive, 1_136_310);
        assert_eq!(selection.variable, "GUST");
        assert_eq!(selection.level, "surface");
        assert_eq!(selection.forecast, "anl");
    }
}
