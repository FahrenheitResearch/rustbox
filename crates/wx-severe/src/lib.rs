use anyhow::{Result, bail};
use sharprs::Profile as SharpProfile;
use sharprs::params::composites::stp_fixed;
use wx_thermo::{ParcelDiagnostics, to_sharprs_profile};
use wx_types::SoundingProfile;

const KTS_TO_MS: f64 = 0.514_444;
const BUNKERS_DEVIATION_MS: f64 = 7.5;
type WindVector = (f64, f64);
type BunkersMotion = (WindVector, WindVector, WindVector);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct KinematicDiagnostics {
    pub srh_01km_m2s2: f64,
    pub srh_03km_m2s2: f64,
    pub bulk_shear_06km_ms: f64,
    pub bunkers_right_u_kts: f64,
    pub bunkers_right_v_kts: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SevereDiagnostics {
    pub significant_tornado_parameter: f64,
    pub kinematics: KinematicDiagnostics,
}

pub fn compute_significant_tornado_parameter(
    profile: &SoundingProfile,
    parcel: &ParcelDiagnostics,
) -> Result<SevereDiagnostics> {
    let sharp_profile = to_sharprs_profile(profile)?;
    let ((storm_u, storm_v), _, _) = calc_bunkers(&sharp_profile)?;
    let srh_01km = calc_helicity(&sharp_profile, 0.0, 1_000.0, storm_u, storm_v)?;
    let srh_03km = calc_helicity(&sharp_profile, 0.0, 3_000.0, storm_u, storm_v)?;
    let bulk_shear_06km_kts = calc_bulk_shear(&sharp_profile, 0.0, 6_000.0)?;
    let bulk_shear_06km_ms = bulk_shear_06km_kts * KTS_TO_MS;

    Ok(SevereDiagnostics {
        significant_tornado_parameter: stp_fixed(
            parcel.surface.cape_jkg,
            parcel.surface.lcl_height_m_agl,
            srh_01km,
            bulk_shear_06km_ms,
        )
        .unwrap_or(0.0),
        kinematics: KinematicDiagnostics {
            srh_01km_m2s2: srh_01km,
            srh_03km_m2s2: srh_03km,
            bulk_shear_06km_ms,
            bunkers_right_u_kts: storm_u,
            bunkers_right_v_kts: storm_v,
        },
    })
}

fn calc_mean_wind_uv(profile: &SharpProfile, bot_agl: f64, top_agl: f64) -> Result<(f64, f64)> {
    let surface_height = profile.hght[profile.sfc];
    let bot_height = surface_height + bot_agl;
    let top_height = surface_height + top_agl;

    let mut sum_u = 0.0;
    let mut sum_v = 0.0;
    let mut count = 0.0;

    for index in 0..profile.pres.len() {
        let height = profile.hght[index];
        let u = profile.u[index];
        let v = profile.v[index];
        if !height.is_finite() || !u.is_finite() || !v.is_finite() {
            continue;
        }
        if height < bot_height || height > top_height {
            continue;
        }

        sum_u += u;
        sum_v += v;
        count += 1.0;
    }

    if count == 0.0 {
        bail!(
            "profile has no valid wind samples between {:.0} and {:.0} m AGL",
            bot_agl,
            top_agl
        );
    }

    Ok((sum_u / count, sum_v / count))
}

fn calc_bunkers(profile: &SharpProfile) -> Result<BunkersMotion> {
    let (mean_u, mean_v) = calc_mean_wind_uv(profile, 0.0, 6_000.0)?;
    let (low_u, low_v) = calc_mean_wind_uv(profile, 0.0, 500.0)?;
    let (upper_u, upper_v) = calc_mean_wind_uv(profile, 5_500.0, 6_000.0)?;

    let shear_u = upper_u - low_u;
    let shear_v = upper_v - low_v;
    let shear_mag = shear_u.hypot(shear_v);
    if !shear_mag.is_finite() || shear_mag <= f64::EPSILON {
        bail!("0-6 km shear is too weak to compute Bunkers motion");
    }

    let deviation_kts = BUNKERS_DEVIATION_MS / KTS_TO_MS;
    let right_mover = (
        mean_u + deviation_kts * shear_v / shear_mag,
        mean_v - deviation_kts * shear_u / shear_mag,
    );
    let left_mover = (
        mean_u - deviation_kts * shear_v / shear_mag,
        mean_v + deviation_kts * shear_u / shear_mag,
    );

    Ok((right_mover, left_mover, (mean_u, mean_v)))
}

fn calc_helicity(
    profile: &SharpProfile,
    bot_agl: f64,
    top_agl: f64,
    storm_u_kts: f64,
    storm_v_kts: f64,
) -> Result<f64> {
    let surface_height = profile.hght[profile.sfc];
    let bot_height = surface_height + bot_agl;
    let top_height = surface_height + top_agl;

    let mut srh = 0.0;
    let mut segment_count = 0usize;

    for index in 0..profile.pres.len().saturating_sub(1) {
        let h0 = profile.hght[index];
        let h1 = profile.hght[index + 1];
        let u0 = profile.u[index];
        let v0 = profile.v[index];
        let u1 = profile.u[index + 1];
        let v1 = profile.v[index + 1];

        if !h0.is_finite()
            || !h1.is_finite()
            || !u0.is_finite()
            || !v0.is_finite()
            || !u1.is_finite()
            || !v1.is_finite()
        {
            continue;
        }
        if h1 < bot_height || h0 > top_height {
            continue;
        }

        let sru0 = (u0 - storm_u_kts) * KTS_TO_MS;
        let srv0 = (v0 - storm_v_kts) * KTS_TO_MS;
        let sru1 = (u1 - storm_u_kts) * KTS_TO_MS;
        let srv1 = (v1 - storm_v_kts) * KTS_TO_MS;

        srh += sru1 * srv0 - sru0 * srv1;
        segment_count += 1;
    }

    if segment_count == 0 {
        bail!(
            "profile has no valid wind segments between {:.0} and {:.0} m AGL",
            bot_agl,
            top_agl
        );
    }

    Ok(srh)
}

fn calc_bulk_shear(profile: &SharpProfile, bot_agl: f64, top_agl: f64) -> Result<f64> {
    let surface_height = profile.hght[profile.sfc];
    let bot_height = surface_height + bot_agl;
    let top_height = surface_height + top_agl;

    let mut bot_index = None;
    let mut top_index = None;

    for index in 0..profile.hght.len() {
        let height = profile.hght[index];
        let u = profile.u[index];
        let v = profile.v[index];
        if !height.is_finite() || !u.is_finite() || !v.is_finite() {
            continue;
        }
        if height <= bot_height {
            bot_index = Some(index);
        }
        if height <= top_height {
            top_index = Some(index);
        }
    }

    let bot_index =
        bot_index.ok_or_else(|| anyhow::anyhow!("missing bottom wind for bulk shear layer"))?;
    let top_index =
        top_index.ok_or_else(|| anyhow::anyhow!("missing top wind for bulk shear layer"))?;

    let shear_u = profile.u[top_index] - profile.u[bot_index];
    let shear_v = profile.v[top_index] - profile.v[bot_index];
    Ok(shear_u.hypot(shear_v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::path::PathBuf;
    use wx_thermo::{compute_parcel_diagnostics, to_sharprs_profile};

    #[derive(Debug, Deserialize)]
    struct ExpectedDiagnostics {
        stp_fixed: f64,
        srh_01km_m2s2: f64,
        srh_03km_m2s2: f64,
        bulk_shear_06km_ms: f64,
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
    fn stp_matches_reference_supercell_environment() {
        let fixture: SoundingFixture = serde_json::from_str(
            &std::fs::read_to_string(fixture_path("sounding_supercell.json"))
                .expect("fixture should be readable"),
        )
        .expect("fixture should parse");
        let parcel =
            compute_parcel_diagnostics(&fixture.profile).expect("parcel diagnostics should work");
        let sharp_profile =
            to_sharprs_profile(&fixture.profile).expect("sharprs profile conversion should work");
        let ((storm_u, storm_v), _, _) =
            calc_bunkers(&sharp_profile).expect("bunkers motion should work");
        let srh_01km =
            calc_helicity(&sharp_profile, 0.0, 1_000.0, storm_u, storm_v).expect("srh should work");
        let srh_03km =
            calc_helicity(&sharp_profile, 0.0, 3_000.0, storm_u, storm_v).expect("srh should work");
        let bulk_shear_06km_ms =
            calc_bulk_shear(&sharp_profile, 0.0, 6_000.0).expect("shear should work") * KTS_TO_MS;
        let severe = compute_significant_tornado_parameter(&fixture.profile, &parcel)
            .expect("severe diagnostics should work");

        assert!(
            (severe.significant_tornado_parameter - fixture.expected.stp_fixed).abs() < 0.5,
            "STP mismatch"
        );
        assert!((srh_01km - fixture.expected.srh_01km_m2s2).abs() < 10.0);
        assert!((srh_03km - fixture.expected.srh_03km_m2s2).abs() < 15.0);
        assert!((bulk_shear_06km_ms - fixture.expected.bulk_shear_06km_ms).abs() < 1.0);
        assert!(severe.kinematics.srh_01km_m2s2 > 100.0);
        assert!(severe.kinematics.bulk_shear_06km_ms > 20.0);
    }
}
