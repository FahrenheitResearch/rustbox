use anyhow::{Result, bail};
use sharprs::Profile as SharpProfile;
use sharprs::params::composites::stp_fixed;
use wx_thermo::{ParcelDiagnostics, to_sharprs_profile};
use wx_types::SoundingProfile;

const KTS_TO_MS: f64 = 0.514_444;
const BUNKERS_DEVIATION_MS: f64 = 7.5;

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
    // The pinned sharprs::winds::helicity call path still fails on the checked-in
    // fixture profiles, so rustbox keeps an exact-layer local port of the winds.rs
    // algorithms until that upstream behavior is reconciled.
    let ((storm_u, storm_v), _, _) = calc_bunkers(&sharp_profile)?;
    let srh_01km = calc_helicity_exact(&sharp_profile, 0.0, 1_000.0, storm_u, storm_v)?;
    let srh_03km = calc_helicity_exact(&sharp_profile, 0.0, 3_000.0, storm_u, storm_v)?;
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

type WindVector = (f64, f64);
type BunkersMotion = (WindVector, WindVector, WindVector);

fn calc_bunkers(profile: &SharpProfile) -> Result<BunkersMotion> {
    let p6km = pressure_at_agl(profile, 6_000.0)?;
    let (mean_u, mean_v) = calc_mean_wind_npw(profile, profile.sfc_pressure(), p6km)?;
    let (shear_u, shear_v) = calc_wind_shear_vector(profile, profile.sfc_pressure(), p6km)?;
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

fn calc_mean_wind_npw(profile: &SharpProfile, pbot: f64, ptop: f64) -> Result<(f64, f64)> {
    let mut sum_u = 0.0;
    let mut sum_v = 0.0;
    let mut count = 0u32;
    let mut pressure = pbot;

    while pressure >= ptop - 0.0001 {
        if let Ok((u, v)) = wind_at_pressure(profile, pressure) {
            sum_u += u;
            sum_v += v;
            count += 1;
        }
        pressure -= 1.0;
    }

    if count == 0 {
        bail!(
            "profile has no valid interpolated wind samples between {:.1} and {:.1} hPa",
            pbot,
            ptop
        );
    }

    Ok((sum_u / count as f64, sum_v / count as f64))
}

fn calc_helicity_exact(
    profile: &SharpProfile,
    lower_agl: f64,
    upper_agl: f64,
    storm_u_kts: f64,
    storm_v_kts: f64,
) -> Result<f64> {
    let lower_pressure = pressure_at_agl(profile, lower_agl)?;
    let upper_pressure = pressure_at_agl(profile, upper_agl)?;
    let (lower_u, lower_v) = wind_at_pressure(profile, lower_pressure)?;
    let (upper_u, upper_v) = wind_at_pressure(profile, upper_pressure)?;

    let mut u_components = vec![lower_u];
    let mut v_components = vec![lower_v];

    for index in 0..profile.pres.len() {
        let pressure = profile.pres[index];
        let u = profile.u[index];
        let v = profile.v[index];
        if !pressure.is_finite() || !u.is_finite() || !v.is_finite() {
            continue;
        }
        if pressure < lower_pressure && pressure > upper_pressure {
            u_components.push(u);
            v_components.push(v);
        }
    }

    u_components.push(upper_u);
    v_components.push(upper_v);

    if u_components.len() < 2 {
        bail!(
            "profile has no valid wind segments between {:.0} and {:.0} m AGL",
            lower_agl,
            upper_agl
        );
    }

    let mut srh = 0.0;
    for index in 0..u_components.len() - 1 {
        let sru0 = (u_components[index] - storm_u_kts) * KTS_TO_MS;
        let srv0 = (v_components[index] - storm_v_kts) * KTS_TO_MS;
        let sru1 = (u_components[index + 1] - storm_u_kts) * KTS_TO_MS;
        let srv1 = (v_components[index + 1] - storm_v_kts) * KTS_TO_MS;
        srh += sru1 * srv0 - sru0 * srv1;
    }

    Ok(srh)
}

fn calc_bulk_shear(profile: &SharpProfile, lower_agl: f64, upper_agl: f64) -> Result<f64> {
    let lower_pressure = pressure_at_agl(profile, lower_agl)?;
    let upper_pressure = pressure_at_agl(profile, upper_agl)?;
    let (lower_u, lower_v, upper_u, upper_v) =
        wind_pair_at_pressures(profile, lower_pressure, upper_pressure)?;
    Ok((upper_u - lower_u).hypot(upper_v - lower_v))
}

fn calc_wind_shear_vector(
    profile: &SharpProfile,
    lower_pressure: f64,
    upper_pressure: f64,
) -> Result<(f64, f64)> {
    let (lower_u, lower_v, upper_u, upper_v) =
        wind_pair_at_pressures(profile, lower_pressure, upper_pressure)?;
    Ok((upper_u - lower_u, upper_v - lower_v))
}

fn wind_pair_at_pressures(
    profile: &SharpProfile,
    lower_pressure: f64,
    upper_pressure: f64,
) -> Result<(f64, f64, f64, f64)> {
    let (lower_u, lower_v) = wind_at_pressure(profile, lower_pressure)?;
    let (upper_u, upper_v) = wind_at_pressure(profile, upper_pressure)?;
    Ok((lower_u, lower_v, upper_u, upper_v))
}

fn pressure_at_agl(profile: &SharpProfile, agl_m: f64) -> Result<f64> {
    if agl_m.abs() <= f64::EPSILON {
        return Ok(profile.sfc_pressure());
    }

    let pressure = profile.pres_at_height(profile.to_msl(agl_m));
    if pressure.is_finite() {
        Ok(pressure)
    } else {
        bail!(
            "profile could not interpolate pressure at {:.0} m AGL",
            agl_m
        )
    }
}

fn wind_at_pressure(profile: &SharpProfile, pressure_hpa: f64) -> Result<(f64, f64)> {
    let (interp_u, interp_v) = profile.interp_wind(pressure_hpa);
    if interp_u.is_finite() && interp_v.is_finite() {
        return Ok((interp_u, interp_v));
    }

    for index in 0..profile.pres.len() {
        let pressure = profile.pres[index];
        let u = profile.u[index];
        let v = profile.v[index];
        if !pressure.is_finite() || !u.is_finite() || !v.is_finite() {
            continue;
        }
        if (pressure - pressure_hpa).abs() <= 0.05 {
            return Ok((u, v));
        }
    }

    bail!(
        "profile field `wind` has no valid data near {:.2} hPa",
        pressure_hpa
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use sharprs::winds;
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
        let srh_01km = calc_helicity_exact(&sharp_profile, 0.0, 1_000.0, storm_u, storm_v)
            .expect("srh should work");
        let srh_03km = calc_helicity_exact(&sharp_profile, 0.0, 3_000.0, storm_u, storm_v)
            .expect("srh should work");
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

    #[test]
    fn pinned_sharprs_helicity_still_fails_on_fixture_profile() {
        let fixture: SoundingFixture = serde_json::from_str(
            &std::fs::read_to_string(fixture_path("sounding_supercell.json"))
                .expect("fixture should be readable"),
        )
        .expect("fixture should parse");
        let sharp_profile =
            to_sharprs_profile(&fixture.profile).expect("sharprs profile conversion should work");
        let (storm_u, storm_v, _, _) =
            winds::non_parcel_bunkers_motion(&sharp_profile).expect("bunkers motion should work");

        let error = winds::helicity(&sharp_profile, 0.0, 1_000.0, storm_u, storm_v, -1.0, true)
            .expect_err("pinned sharprs helicity path is expected to fail on this fixture");

        assert!(
            error.to_string().contains("wind"),
            "unexpected sharprs::winds failure: {error}"
        );
    }
}
