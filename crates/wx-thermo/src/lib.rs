use anyhow::Result;
use sharprs::Profile as SharpProfile;
use sharprs::params::cape::{
    ParcelResult, ParcelType, Profile as CapeProfile, define_parcel, parcelx,
};
use sharprs::profile::StationInfo;
use wx_types::SoundingProfile;

#[derive(Debug, Clone, PartialEq)]
pub struct ParcelSnapshot {
    pub cape_jkg: f64,
    pub cin_jkg: f64,
    pub lcl_pressure_hpa: f64,
    pub lcl_height_m_agl: f64,
    pub lfc_pressure_hpa: Option<f64>,
    pub el_pressure_hpa: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParcelDiagnostics {
    pub surface: ParcelSnapshot,
    pub mixed_layer: ParcelSnapshot,
    pub most_unstable: ParcelSnapshot,
    pub ecape_jkg: Option<f64>,
}

pub trait EcapeAdapter {
    fn compute_ecape(
        &self,
        profile: &SoundingProfile,
        diagnostics: &ParcelDiagnostics,
    ) -> Result<Option<f64>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoEcape;

impl EcapeAdapter for NoEcape {
    fn compute_ecape(
        &self,
        _profile: &SoundingProfile,
        _diagnostics: &ParcelDiagnostics,
    ) -> Result<Option<f64>> {
        // TODO: replace this default adapter with an ecape-rs-backed implementation
        // for the model-derived HRRR column path once the input mapping is settled.
        Ok(None)
    }
}

pub fn compute_parcel_diagnostics(profile: &SoundingProfile) -> Result<ParcelDiagnostics> {
    compute_parcel_diagnostics_with_ecape(profile, NoEcape)
}

pub fn compute_parcel_diagnostics_with_ecape<A: EcapeAdapter>(
    profile: &SoundingProfile,
    ecape: A,
) -> Result<ParcelDiagnostics> {
    let cape_profile = to_cape_profile(profile)?;

    let surface = parcelx(
        &cape_profile,
        &define_parcel(&cape_profile, ParcelType::Surface),
        None,
        None,
    );
    let mixed_layer = parcelx(
        &cape_profile,
        &define_parcel(&cape_profile, ParcelType::MixedLayer { depth_hpa: 100.0 }),
        None,
        None,
    );
    let most_unstable = parcelx(
        &cape_profile,
        &define_parcel(&cape_profile, ParcelType::MostUnstable { depth_hpa: 300.0 }),
        None,
        None,
    );

    let mut diagnostics = ParcelDiagnostics {
        surface: snapshot_from_parcel(&surface),
        mixed_layer: snapshot_from_parcel(&mixed_layer),
        most_unstable: snapshot_from_parcel(&most_unstable),
        ecape_jkg: None,
    };
    diagnostics.ecape_jkg = ecape.compute_ecape(profile, &diagnostics)?;

    Ok(diagnostics)
}

pub fn to_sharprs_profile(profile: &SoundingProfile) -> Result<SharpProfile> {
    let pres: Vec<f64> = profile
        .levels
        .iter()
        .map(|level| level.pressure_hpa)
        .collect();
    let hght: Vec<f64> = profile.levels.iter().map(|level| level.height_m).collect();
    let tmpc: Vec<f64> = profile
        .levels
        .iter()
        .map(|level| level.temperature_c)
        .collect();
    let dwpc: Vec<f64> = profile
        .levels
        .iter()
        .map(|level| level.dewpoint_c)
        .collect();
    let wdir: Vec<f64> = profile
        .levels
        .iter()
        .map(|level| level.wind_direction_deg)
        .collect();
    let wspd: Vec<f64> = profile
        .levels
        .iter()
        .map(|level| level.wind_speed_kts)
        .collect();

    SharpProfile::new(
        &pres,
        &hght,
        &tmpc,
        &dwpc,
        &wdir,
        &wspd,
        &[],
        StationInfo {
            station_id: profile.station_id.clone(),
            latitude: profile.latitude.unwrap_or(f64::NAN),
            longitude: profile.longitude.unwrap_or(f64::NAN),
            elevation: profile
                .levels
                .first()
                .map(|level| level.height_m)
                .unwrap_or(f64::NAN),
            datetime: profile
                .valid_time
                .map(|value| value.to_rfc3339())
                .unwrap_or_default(),
        },
    )
    .map_err(|error| anyhow::anyhow!(error))
}

fn to_cape_profile(profile: &SoundingProfile) -> Result<CapeProfile> {
    if profile.levels.len() < 2 {
        return Err(anyhow::anyhow!(
            "sounding profile needs at least two levels"
        ));
    }

    Ok(CapeProfile::new(
        profile
            .levels
            .iter()
            .map(|level| level.pressure_hpa)
            .collect(),
        profile.levels.iter().map(|level| level.height_m).collect(),
        profile
            .levels
            .iter()
            .map(|level| level.temperature_c)
            .collect(),
        profile
            .levels
            .iter()
            .map(|level| level.dewpoint_c)
            .collect(),
        0,
    ))
}

fn snapshot_from_parcel(parcel: &ParcelResult) -> ParcelSnapshot {
    ParcelSnapshot {
        cape_jkg: finite_or_zero(parcel.bplus),
        cin_jkg: finite_or_zero(parcel.bminus),
        lcl_pressure_hpa: finite_or_zero(parcel.lclpres),
        lcl_height_m_agl: finite_or_zero(parcel.lclhght),
        lfc_pressure_hpa: finite_option(parcel.lfcpres),
        el_pressure_hpa: finite_option(parcel.elpres),
    }
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

fn finite_option(value: f64) -> Option<f64> {
    if value.is_finite() { Some(value) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::path::PathBuf;

    #[derive(Debug, Deserialize)]
    struct ExpectedDiagnostics {
        sbcape_jkg: f64,
        sbcin_jkg: f64,
        sblcl_m_agl: f64,
        mlcape_jkg: f64,
        mlcin_jkg: f64,
        mucape_jkg: f64,
        mucin_jkg: f64,
    }

    #[derive(Debug, Deserialize)]
    struct SoundingFixture {
        #[serde(flatten)]
        profile: SoundingProfile,
        expected: ExpectedDiagnostics,
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    #[test]
    fn parcel_diagnostics_match_reference_soundings() {
        let fixture: SoundingFixture = serde_json::from_str(
            &std::fs::read_to_string(fixture_path("sounding_supercell.json"))
                .expect("fixture should be readable"),
        )
        .expect("fixture should parse");

        let diagnostics =
            compute_parcel_diagnostics(&fixture.profile).expect("parcel diagnostics should work");
        println!(
            "SBCAPE={:.3} SBCIN={:.3} SBLCL={:.3} MLCAPE={:.3} MLCIN={:.3} MUCAPE={:.3} MUCIN={:.3}",
            diagnostics.surface.cape_jkg,
            diagnostics.surface.cin_jkg,
            diagnostics.surface.lcl_height_m_agl,
            diagnostics.mixed_layer.cape_jkg,
            diagnostics.mixed_layer.cin_jkg,
            diagnostics.most_unstable.cape_jkg,
            diagnostics.most_unstable.cin_jkg,
        );

        assert!(
            (diagnostics.surface.cape_jkg - fixture.expected.sbcape_jkg).abs() < 25.0,
            "SBCAPE mismatch"
        );
        assert!(
            (diagnostics.surface.cin_jkg - fixture.expected.sbcin_jkg).abs() < 10.0,
            "SBCIN mismatch"
        );
        assert!(
            (diagnostics.surface.lcl_height_m_agl - fixture.expected.sblcl_m_agl).abs() < 50.0,
            "SBLCL mismatch"
        );
        assert!(
            (diagnostics.mixed_layer.cape_jkg - fixture.expected.mlcape_jkg).abs() < 25.0,
            "MLCAPE mismatch"
        );
        assert!(
            (diagnostics.mixed_layer.cin_jkg - fixture.expected.mlcin_jkg).abs() < 10.0,
            "MLCIN mismatch"
        );
        assert!(
            (diagnostics.most_unstable.cape_jkg - fixture.expected.mucape_jkg).abs() < 25.0,
            "MUCAPE mismatch"
        );
        assert!(
            (diagnostics.most_unstable.cin_jkg - fixture.expected.mucin_jkg).abs() < 10.0,
            "MUCIN mismatch"
        );
    }

    #[test]
    fn default_ecape_adapter_returns_none() {
        let fixture: SoundingFixture = serde_json::from_str(
            &std::fs::read_to_string(fixture_path("sounding_supercell.json"))
                .expect("fixture should be readable"),
        )
        .expect("fixture should parse");

        let diagnostics =
            compute_parcel_diagnostics(&fixture.profile).expect("parcel diagnostics should work");

        assert_eq!(diagnostics.ecape_jkg, None);
    }
}
