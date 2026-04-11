use crate::colormap::{Extend, LeveledColormap};
use crate::colormaps;
use anyhow::{Result, bail};
use image::Rgba;

#[derive(Clone, Debug)]
pub struct RenderStyle {
    pub id: String,
    pub tick_step: Option<f64>,
    pub colormap: LeveledColormap,
}

impl RenderStyle {
    pub fn color_for_value(&self, value: f64) -> Rgba<u8> {
        self.colormap.map(value)
    }

    pub fn normalized_value(&self, value: f64) -> Option<f32> {
        let (min_value, max_value) = self.colormap.range()?;
        let span = (max_value - min_value).abs();
        if span < f64::EPSILON {
            return Some(1.0);
        }
        Some(((value - min_value) / (max_value - min_value)).clamp(0.0, 1.0) as f32)
    }

    pub fn tick_values(&self) -> Vec<f64> {
        let Some((min_value, max_value)) = self.colormap.range() else {
            return Vec::new();
        };
        if let Some(step) = self.tick_step.filter(|value| *value > 0.0) {
            let start = (min_value / step).ceil() * step;
            let mut ticks = Vec::new();
            let mut value = start;
            while value <= max_value + step * 0.01 {
                ticks.push(round_tick(value, step));
                value += step;
            }
            if ticks
                .first()
                .is_none_or(|value| (*value - min_value).abs() > step * 0.25)
            {
                ticks.insert(0, min_value);
            }
            if ticks
                .last()
                .is_none_or(|value| (*value - max_value).abs() > step * 0.25)
            {
                ticks.push(max_value);
            }
            ticks
        } else {
            let steps = 5usize;
            (0..steps)
                .map(|index| {
                    let fraction = index as f64 / (steps.saturating_sub(1)) as f64;
                    min_value + (max_value - min_value) * fraction
                })
                .collect()
        }
    }
}

pub fn resolve_render_style(
    style_id: &str,
    value_range_override: Option<(f32, f32)>,
) -> Result<RenderStyle> {
    let definition = style_definition(style_id)?;
    let levels = match value_range_override {
        Some((min_value, max_value)) if (max_value - min_value).abs() > f32::EPSILON => {
            linear_levels(min_value as f64, max_value as f64, 24)
        }
        _ => definition.levels,
    };
    let colormap = LeveledColormap::from_palette(
        &definition.palette,
        &levels,
        definition.extend,
        definition.mask_below,
    );

    Ok(RenderStyle {
        id: style_id.to_string(),
        tick_step: definition.tick_step,
        colormap,
    })
}

pub fn format_tick(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= 100.0 {
        format!("{value:.0}")
    } else if magnitude >= 1.0 {
        let rounded = value.round();
        if (value - rounded).abs() < 1e-6 {
            format!("{rounded:.0}")
        } else {
            format!("{value:.1}")
        }
    } else if magnitude >= 0.01 {
        format!("{value:.2}")
    } else if magnitude >= 0.001 {
        format!("{value:.3}")
    } else if magnitude >= 0.0001 {
        format!("{value:.4}")
    } else {
        format!("{value:.1e}")
    }
}

struct StyleDefinition {
    palette: Vec<Rgba<u8>>,
    levels: Vec<f64>,
    extend: Extend,
    mask_below: Option<f64>,
    tick_step: Option<f64>,
}

fn style_definition(style_id: &str) -> Result<StyleDefinition> {
    match style_id {
        "winds" | "gust" => Ok(StyleDefinition {
            palette: colormaps::winds(64),
            levels: inclusive_levels(0.0, 80.0, 2.5),
            extend: Extend::Max,
            mask_below: None,
            tick_step: Some(10.0),
        }),
        "temperature" | "tmp" | "theta" => Ok(StyleDefinition {
            palette: colormaps::temperature(180),
            levels: inclusive_levels(-40.0, 120.0, 2.0),
            extend: Extend::Both,
            mask_below: None,
            tick_step: Some(10.0),
        }),
        "reflectivity" | "dbz" => Ok(StyleDefinition {
            palette: colormaps::reflectivity(),
            levels: inclusive_levels(5.0, 70.0, 2.5),
            extend: Extend::Both,
            mask_below: Some(5.0),
            tick_step: Some(5.0),
        }),
        "cape" => Ok(StyleDefinition {
            palette: colormaps::cape(),
            levels: inclusive_levels(0.0, 8000.0, 100.0),
            extend: Extend::Both,
            mask_below: None,
            tick_step: Some(500.0),
        }),
        "srh" => Ok(StyleDefinition {
            palette: colormaps::srh(),
            levels: inclusive_levels(0.0, 1000.0, 10.0),
            extend: Extend::Both,
            mask_below: None,
            tick_step: Some(50.0),
        }),
        "stp" => Ok(StyleDefinition {
            palette: colormaps::stp(),
            levels: inclusive_levels(0.0, 10.0, 0.1),
            extend: Extend::Both,
            mask_below: None,
            tick_step: Some(1.0),
        }),
        "vorticity" => Ok(StyleDefinition {
            palette: colormaps::relative_vorticity(128),
            levels: inclusive_levels(-6.0e-4, 6.0e-4, 2.0e-5),
            extend: Extend::Both,
            mask_below: None,
            tick_step: Some(1.0e-4),
        }),
        "divergence" => Ok(StyleDefinition {
            palette: colormaps::divergence(96),
            levels: inclusive_levels(-4.0e-4, 4.0e-4, 2.0e-5),
            extend: Extend::Both,
            mask_below: None,
            tick_step: Some(1.0e-4),
        }),
        "advection" => Ok(StyleDefinition {
            palette: colormaps::advection(96),
            levels: inclusive_levels(-2.0e-4, 2.0e-4, 1.0e-5),
            extend: Extend::Both,
            mask_below: None,
            tick_step: Some(5.0e-5),
        }),
        "frontogenesis" => Ok(StyleDefinition {
            palette: colormaps::frontogenesis(96),
            levels: inclusive_levels(-3.0e-7, 3.0e-7, 1.0e-8),
            extend: Extend::Both,
            mask_below: None,
            tick_step: Some(1.0e-7),
        }),
        other => bail!("unsupported render style '{}'", other),
    }
}

fn inclusive_levels(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let mut levels = Vec::new();
    let mut value = start;
    while value <= stop + step * 0.01 {
        levels.push(value);
        value += step;
    }
    if levels.last().is_none_or(|value| *value < stop) {
        levels.push(stop);
    }
    levels
}

fn linear_levels(min_value: f64, max_value: f64, intervals: usize) -> Vec<f64> {
    if intervals == 0 || (max_value - min_value).abs() < f64::EPSILON {
        return vec![min_value, max_value.max(min_value + 1.0)];
    }
    (0..=intervals)
        .map(|index| min_value + (max_value - min_value) * index as f64 / intervals as f64)
        .collect()
}

fn round_tick(value: f64, step: f64) -> f64 {
    if step.abs() >= 1.0 {
        value.round()
    } else if step.abs() >= 0.1 {
        (value * 10.0).round() / 10.0
    } else if step.abs() >= 0.01 {
        (value * 100.0).round() / 100.0
    } else if step.abs() >= 0.001 {
        (value * 1000.0).round() / 1000.0
    } else if step.abs() >= 0.0001 {
        (value * 10000.0).round() / 10000.0
    } else {
        value
    }
}
