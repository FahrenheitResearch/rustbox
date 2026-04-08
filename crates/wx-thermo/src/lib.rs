use wx_types::SoundingProfile;

#[derive(Debug, Clone, PartialEq)]
pub struct ParcelRequest {
    pub mixed_layer_depth_m: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParcelDiagnostics {
    pub sbcape_jkg: f32,
    pub mlcape_jkg: f32,
    pub mucape_jkg: f32,
    pub cin_jkg: f32,
}

pub fn compute_surface_parcel(profile: &SoundingProfile, request: &ParcelRequest) -> ParcelDiagnostics {
    let levels = profile.levels.len() as f32;
    let base = request.mixed_layer_depth_m.max(1.0) / 10.0;
    ParcelDiagnostics {
        sbcape_jkg: levels * base,
        mlcape_jkg: levels * base * 0.8,
        mucape_jkg: levels * base * 1.2,
        cin_jkg: -(levels * 2.0),
    }
}

