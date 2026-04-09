use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HrrrSourceCandidate {
    pub name: String,
    pub priority: u8,
    pub grib_url: String,
    pub idx_url: String,
}

pub fn hrrr_source_candidates(
    cycle: DateTime<Utc>,
    product: &str,
    forecast_hour: u16,
) -> Vec<HrrrSourceCandidate> {
    let date = cycle.format("%Y%m%d").to_string();
    let hour = cycle.format("%H").to_string();
    let product_code = hrrr_product_code(product);

    let mut candidates = vec![
        candidate(
            "nomads",
            1,
            format!(
                "https://nomads.ncep.noaa.gov/pub/data/nccf/com/hrrr/prod/hrrr.{date}/conus/hrrr.t{hour}z.{product_code}f{forecast_hour:02}.grib2"
            ),
        ),
        candidate(
            "aws",
            2,
            format!(
                "https://noaa-hrrr-bdp-pds.s3.amazonaws.com/hrrr.{date}/conus/hrrr.t{hour}z.{product_code}f{forecast_hour:02}.grib2"
            ),
        ),
        candidate(
            "google",
            3,
            format!(
                "https://storage.googleapis.com/high-resolution-rapid-refresh/hrrr.{date}/conus/hrrr.t{hour}z.{product_code}f{forecast_hour:02}.grib2"
            ),
        ),
        candidate(
            "azure",
            4,
            format!(
                "https://noaahrrr.blob.core.windows.net/hrrr/hrrr.{date}/conus/hrrr.t{hour}z.{product_code}f{forecast_hour:02}.grib2"
            ),
        ),
    ];
    candidates.sort_by_key(|candidate| candidate.priority);
    candidates
}

fn candidate(name: &str, priority: u8, grib_url: String) -> HrrrSourceCandidate {
    HrrrSourceCandidate {
        name: name.to_string(),
        priority,
        idx_url: format!("{grib_url}.idx"),
        grib_url,
    }
}

fn hrrr_product_code(product: &str) -> &'static str {
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

    #[test]
    fn hrrr_candidates_follow_expected_priority() {
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let candidates = hrrr_source_candidates(cycle, "sfc", 0);
        assert_eq!(candidates.len(), 4);
        assert_eq!(candidates[0].name, "nomads");
        assert_eq!(candidates[1].name, "aws");
        assert!(candidates[1].grib_url.contains("wrfsfcf00"));
        assert!(candidates[2].idx_url.ends_with(".idx"));
    }
}
