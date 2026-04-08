#[derive(Debug, Clone, PartialEq)]
pub struct RadarVolume {
    pub site: String,
    pub sweep_count: usize,
}

impl RadarVolume {
    pub fn summary(&self) -> String {
        format!("{} sweeps from {}", self.sweep_count, self.site)
    }
}
