use image::Rgba;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extend {
    Neither,
    Min,
    Max,
    Both,
}

#[derive(Clone, Debug)]
pub struct LeveledColormap {
    pub levels: Vec<f64>,
    pub colors: Vec<Rgba<u8>>,
    pub under_color: Option<Rgba<u8>>,
    pub over_color: Option<Rgba<u8>>,
    pub mask_below: Option<f64>,
}

impl LeveledColormap {
    pub fn map(&self, value: f64) -> Rgba<u8> {
        if !value.is_finite() {
            return Rgba([0, 0, 0, 0]);
        }
        if let Some(mask_below) = self.mask_below
            && value < mask_below
        {
            return Rgba([0, 0, 0, 0]);
        }
        if self.levels.len() < 2 || self.colors.is_empty() {
            return Rgba([0, 0, 0, 0]);
        }
        if value < self.levels[0] {
            return self.under_color.unwrap_or(Rgba([0, 0, 0, 0]));
        }
        let interval_count = self.levels.len() - 1;
        for index in 0..interval_count {
            if value < self.levels[index + 1] {
                return self.colors[index.min(self.colors.len().saturating_sub(1))];
            }
        }
        if value >= self.levels[interval_count] {
            return self
                .over_color
                .unwrap_or(self.colors[self.colors.len().saturating_sub(1)]);
        }
        self.colors[self.colors.len().saturating_sub(1)]
    }

    pub fn from_palette(
        palette: &[Rgba<u8>],
        levels: &[f64],
        extend: Extend,
        mask_below: Option<f64>,
    ) -> Self {
        let interval_count = levels.len().saturating_sub(1);
        if interval_count == 0 || palette.is_empty() {
            return Self {
                levels: levels.to_vec(),
                colors: Vec::new(),
                under_color: None,
                over_color: None,
                mask_below,
            };
        }

        let sampled: Vec<Rgba<u8>> = (0..interval_count)
            .map(|index| {
                let t = if interval_count <= 1 {
                    0.5
                } else {
                    index as f64 / (interval_count - 1) as f64
                };
                let palette_index = (t * (palette.len().saturating_sub(1)) as f64).round() as usize;
                palette[palette_index.min(palette.len().saturating_sub(1))]
            })
            .collect();

        let under_color = match extend {
            Extend::Min | Extend::Both => sampled.first().copied(),
            Extend::Neither | Extend::Max => None,
        };
        let over_color = match extend {
            Extend::Max | Extend::Both => sampled.last().copied(),
            Extend::Neither | Extend::Min => None,
        };

        Self {
            levels: levels.to_vec(),
            colors: sampled,
            under_color,
            over_color,
            mask_below,
        }
    }

    pub fn range(&self) -> Option<(f64, f64)> {
        if self.levels.len() >= 2 {
            Some((self.levels[0], self.levels[self.levels.len() - 1]))
        } else {
            None
        }
    }
}
