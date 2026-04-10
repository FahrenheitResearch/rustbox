use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub mod nexrad;

pub use nexrad::color_table::{ColorTable, ColorTablePreset, ColorTableSelection};
pub use nexrad::derived::DerivedProducts;
pub use nexrad::detection::{
    HailDetection, HailIndicator, MesocycloneDetection, RotationDetector, RotationSense,
    RotationStrength, TVSDetection,
};
pub use nexrad::level2::{Level2File, Level2Sweep, MomentData, RadialData};
pub use nexrad::products::RadarProduct;
pub use nexrad::render::{RadarRenderer, RenderMode, RenderedSweep};
pub use nexrad::sites::{RADAR_SITES, RadarSite, all_site_ids, find_nearest_site, find_site};
pub use nexrad::srv::SRVComputer;

const DEFAULT_ECHO_TOPS_THRESHOLD_DBZ: f32 = 18.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadarVolumeSummary {
    pub station_id: String,
    pub site_name: Option<String>,
    pub timestamp_utc: String,
    pub vcp: Option<u16>,
    pub vcp_description: Option<String>,
    pub sweep_count: usize,
    pub partial: bool,
    pub products: Vec<String>,
    pub lowest_elevation_deg: Option<f32>,
    pub highest_elevation_deg: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectionSummary {
    pub station_id: String,
    pub mesocyclone_count: usize,
    pub tvs_count: usize,
    pub hail_count: usize,
    pub storm_motion_direction_from_deg: Option<f32>,
    pub storm_motion_speed_kt: Option<f32>,
}

pub fn parse_level2_bytes(raw_data: &[u8]) -> Result<Level2File> {
    Level2File::parse(raw_data)
}

pub fn read_level2_file(path: impl AsRef<Path>) -> Result<Level2File> {
    let path = path.as_ref();
    let raw = std::fs::read(path)
        .with_context(|| format!("failed to read Level II file {}", path.display()))?;
    parse_level2_bytes(&raw)
}

pub fn site_for_volume(file: &Level2File) -> Result<&'static RadarSite> {
    find_site(&file.station_id).ok_or_else(|| anyhow!("unknown radar site {}", file.station_id))
}

pub fn summarize_volume(file: &Level2File) -> RadarVolumeSummary {
    let site_name = find_site(&file.station_id).map(|site| site.name.to_string());
    let mut products = BTreeSet::new();
    let mut lowest_elevation_deg: Option<f32> = None;
    let mut highest_elevation_deg: Option<f32> = None;

    for sweep in &file.sweeps {
        lowest_elevation_deg = Some(
            lowest_elevation_deg
                .map(|value| value.min(sweep.elevation_angle))
                .unwrap_or(sweep.elevation_angle),
        );
        highest_elevation_deg = Some(
            highest_elevation_deg
                .map(|value| value.max(sweep.elevation_angle))
                .unwrap_or(sweep.elevation_angle),
        );
        for radial in &sweep.radials {
            for moment in &radial.moments {
                products.insert(moment.product.short_name().to_string());
            }
        }
    }

    RadarVolumeSummary {
        station_id: file.station_id.clone(),
        site_name,
        timestamp_utc: file.timestamp_string(),
        vcp: file.vcp,
        vcp_description: file.vcp_description().map(str::to_string),
        sweep_count: file.sweeps.len(),
        partial: file.partial,
        products: products.into_iter().collect(),
        lowest_elevation_deg,
        highest_elevation_deg,
    }
}

pub fn available_products(file: &Level2File) -> Vec<RadarProduct> {
    let mut products = BTreeSet::new();
    for sweep in &file.sweeps {
        for radial in &sweep.radials {
            for moment in &radial.moments {
                products.insert(moment.product.short_name().to_string());
            }
        }
    }

    let mut parsed: Vec<RadarProduct> = products
        .into_iter()
        .map(|name| RadarProduct::from_name(&name))
        .filter(|product| *product != RadarProduct::Unknown)
        .collect();
    parsed.sort_by_key(|product| product.short_name().to_string());
    parsed
}

pub fn derive_product_sweep(
    file: &Level2File,
    product: RadarProduct,
    sweep_index: usize,
) -> Result<Level2Sweep> {
    match product {
        RadarProduct::VIL => Ok(DerivedProducts::compute_vil(file)),
        RadarProduct::EchoTops => Ok(DerivedProducts::compute_echo_tops(
            file,
            DEFAULT_ECHO_TOPS_THRESHOLD_DBZ,
        )),
        RadarProduct::StormRelativeVelocity => {
            let velocity_sweeps: Vec<&Level2Sweep> = file
                .sweeps
                .iter()
                .filter(|sweep| {
                    sweep
                        .radials
                        .iter()
                        .flat_map(|radial| &radial.moments)
                        .any(|moment| moment.product == RadarProduct::Velocity)
                })
                .collect();
            if velocity_sweeps.is_empty() {
                bail!("volume does not contain velocity sweeps");
            }
            let sweep = velocity_sweeps
                .get(sweep_index)
                .copied()
                .with_context(|| format!("velocity sweep index {} out of range", sweep_index))?;
            let (direction_from_deg, speed_kt) =
                SRVComputer::estimate_storm_motion(&velocity_sweeps);
            Ok(SRVComputer::compute(sweep, direction_from_deg, speed_kt))
        }
        _ => {
            let sweep = find_sweep_for_product(file, product.base_product(), sweep_index)
                .with_context(|| {
                    format!(
                        "product {} not available for sweep index {}",
                        product.short_name(),
                        sweep_index
                    )
                })?;
            Ok(sweep.clone())
        }
    }
}

pub fn render_product(
    file: &Level2File,
    product: RadarProduct,
    sweep_index: usize,
    image_size: u32,
    mode: RenderMode,
    preset: ColorTablePreset,
) -> Result<RenderedSweep> {
    let site = site_for_volume(file)?;
    let sweep = derive_product_sweep(file, product, sweep_index)?;
    let color_table = ColorTable::for_product_preset(product, preset);
    let rendered = match mode {
        RenderMode::Classic => {
            RadarRenderer::render_sweep_with_table(&sweep, product, site, image_size, &color_table)
        }
        RenderMode::Smooth => {
            RadarRenderer::render_sweep_smooth(&sweep, product, site, image_size, &color_table)
        }
    };
    rendered.with_context(|| {
        format!(
            "failed to render {} for radar site {}",
            product.short_name(),
            file.station_id
        )
    })
}

pub fn detect_signatures(file: &Level2File) -> Result<DetectionSummary> {
    let site = site_for_volume(file)?;
    let velocity_sweeps: Vec<&Level2Sweep> = file
        .sweeps
        .iter()
        .filter(|sweep| {
            sweep
                .radials
                .iter()
                .flat_map(|radial| &radial.moments)
                .any(|moment| moment.product == RadarProduct::Velocity)
        })
        .collect();
    let storm_motion = if velocity_sweeps.is_empty() {
        None
    } else {
        Some(SRVComputer::estimate_storm_motion(&velocity_sweeps))
    };
    let (mesocyclones, tvs, hail) = RotationDetector::detect(file, site);
    Ok(DetectionSummary {
        station_id: file.station_id.clone(),
        mesocyclone_count: mesocyclones.len(),
        tvs_count: tvs.len(),
        hail_count: hail.len(),
        storm_motion_direction_from_deg: storm_motion.map(|(dir, _)| dir),
        storm_motion_speed_kt: storm_motion.map(|(_, speed)| speed),
    })
}

fn find_sweep_for_product(
    file: &Level2File,
    product: RadarProduct,
    sweep_index: usize,
) -> Option<&Level2Sweep> {
    file.sweeps
        .iter()
        .filter(|sweep| {
            sweep
                .radials
                .iter()
                .flat_map(|radial| &radial.moments)
                .any(|moment| moment.product == product)
        })
        .nth(sweep_index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    const RADAR_FIXTURE: &str = "KATX20240101_000258_partial_V06";

    #[test]
    fn parses_level2_fixture_and_summarizes_products() {
        let file = read_level2_file(fixture_path(RADAR_FIXTURE)).expect("fixture parses");
        let summary = summarize_volume(&file);
        assert_eq!(summary.station_id, "KATX");
        assert!(summary.sweep_count >= 3);
        assert!(summary.products.iter().any(|name| name == "REF"));
        assert!(summary.products.iter().any(|name| name == "VEL"));
    }

    #[test]
    fn renders_reflectivity_fixture_to_non_empty_rgba() {
        let file = read_level2_file(fixture_path(RADAR_FIXTURE)).expect("fixture parses");
        let rendered = render_product(
            &file,
            RadarProduct::Reflectivity,
            0,
            256,
            RenderMode::Classic,
            ColorTablePreset::Default,
        )
        .expect("reflectivity render succeeds");
        assert_eq!(rendered.width, 256);
        assert_eq!(rendered.height, 256);
        assert_eq!(rendered.pixels.len(), 256 * 256 * 4);
        assert!(rendered.pixels.iter().any(|value| *value != 0));
    }

    #[test]
    fn computes_derived_products_and_detection_summary() {
        let file = read_level2_file(fixture_path(RADAR_FIXTURE)).expect("fixture parses");
        let vil = derive_product_sweep(&file, RadarProduct::VIL, 0).expect("VIL derives");
        assert!(!vil.radials.is_empty());
        let echo_tops =
            derive_product_sweep(&file, RadarProduct::EchoTops, 0).expect("echo tops derive");
        assert!(!echo_tops.radials.is_empty());
        let summary = detect_signatures(&file).expect("detection runs");
        assert_eq!(summary.station_id, "KATX");
        assert!(summary.storm_motion_direction_from_deg.is_some());
        assert!(summary.storm_motion_speed_kt.is_some());
    }
}
