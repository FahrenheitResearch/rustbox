use wx_types::Field2D;

#[derive(Debug, Clone, PartialEq)]
pub struct LayerKinematics {
    pub srh_01km_m2s2: f32,
    pub srh_03km_m2s2: f32,
    pub bulk_shear_06km_ms: f32,
}

pub fn summarize_grid(field: &Field2D) -> Option<(f32, f32)> {
    let min = field.values.iter().copied().reduce(f32::min)?;
    let max = field.values.iter().copied().reduce(f32::max)?;
    Some((min, max))
}
