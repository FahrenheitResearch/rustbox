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
    levels_override: Option<&[f64]>,
    tick_step_override: Option<f64>,
) -> Result<RenderStyle> {
    let definition = style_definition(style_id)?;
    let levels = if let Some(levels) = levels_override.filter(|levels| levels.len() >= 2) {
        levels.to_vec()
    } else if let Some((min_value, max_value)) = value_range_override
        .filter(|(min_value, max_value)| (max_value - min_value).abs() > f32::EPSILON)
    {
        linear_levels(min_value as f64, max_value as f64, 24)
    } else {
        definition.levels
    };
    let colormap = LeveledColormap::from_palette(
        &definition.palette,
        &levels,
        definition.extend,
        definition.mask_below,
    );

    Ok(RenderStyle {
        id: style_id.to_string(),
        tick_step: tick_step_override.or(definition.tick_step),
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
        "winds" | "gust" => Ok(make_style(
            colormaps::winds(64),
            inclusive_levels(0.0, 80.0, 2.5),
            Extend::Max,
            None,
            Some(10.0),
        )),
        "wspd" | "wspd10" => Ok(make_style(
            colormaps::winds(64),
            inclusive_levels(0.0, 62.0, 2.0),
            Extend::Max,
            None,
            Some(5.0),
        )),
        "ua" | "va" | "shear_0_1km" | "shear_0_6km" | "bulk_shear" | "effective_inflow" => {
            Ok(make_style(
                colormaps::winds(64),
                inclusive_levels(0.0, 120.0, 5.0),
                Extend::Max,
                None,
                Some(10.0),
            ))
        }
        "wdir" | "wdir10" => Ok(make_style(
            colormaps::winds(64),
            inclusive_levels(0.0, 360.0, 10.0),
            Extend::Neither,
            None,
            Some(10.0),
        )),
        "temperature" | "tmp" | "theta" | "theta_e" | "tv" | "twb" | "theta_w" | "tc" => {
            Ok(make_style(
                colormaps::temperature(180),
                inclusive_levels(-60.0, 120.0, 2.0),
                Extend::Both,
                None,
                Some(10.0),
            ))
        }
        "dewpoint" | "dpt" | "td" | "dp2m" => Ok(make_style(
            colormaps::dewpoint(80, 50),
            inclusive_levels(-40.0, 82.0, 2.0),
            Extend::Both,
            None,
            Some(10.0),
        )),
        "pw" => Ok(make_style(
            colormaps::dewpoint(80, 50),
            inclusive_levels(0.0, 66.0, 3.0),
            Extend::Max,
            None,
            Some(3.0),
        )),
        "mixing_ratio" | "specific_humidity" => Ok(make_style(
            colormaps::dewpoint(80, 50),
            inclusive_levels(0.0, 30.0, 1.0),
            Extend::Max,
            None,
            Some(5.0),
        )),
        "rh" | "rh2m" | "cloudfrac" => Ok(make_style(
            colormaps::rh(),
            inclusive_levels(0.0, 100.0, 5.0),
            Extend::Neither,
            None,
            Some(10.0),
        )),
        "reflectivity" | "dbz" | "maxdbz" => Ok(make_style(
            colormaps::reflectivity(),
            inclusive_levels(5.0, 70.0, 2.5),
            Extend::Max,
            Some(5.0),
            Some(5.0),
        )),
        "cape" | "sbcape" | "mlcape" | "mucape" | "effective_cape" => Ok(make_style(
            colormaps::cape(),
            inclusive_levels(0.0, 4250.0, 250.0),
            Extend::Max,
            None,
            Some(500.0),
        )),
        "three_cape" | "cape3d" => Ok(make_style(
            colormaps::three_cape(),
            inclusive_levels(0.0, 4250.0, 250.0),
            Extend::Max,
            None,
            Some(500.0),
        )),
        "cin" | "sbcin" | "mlcin" | "mucin" => Ok(make_style(
            colormaps::cape(),
            inclusive_levels(-300.0, 0.0, 25.0),
            Extend::Min,
            None,
            Some(25.0),
        )),
        "srh" | "srh1" | "srh3" | "effective_srh" => Ok(make_style(
            colormaps::srh(),
            inclusive_levels(0.0, 525.0, 25.0),
            Extend::Max,
            None,
            Some(50.0),
        )),
        "stp" | "stp_fixed" | "stp_effective" => Ok(make_style(
            colormaps::stp(),
            inclusive_levels(0.0, 10.0, 1.0),
            Extend::Max,
            None,
            Some(1.0),
        )),
        "ehi" => Ok(make_style(
            colormaps::ehi(),
            inclusive_levels(0.0, 5.5, 0.5),
            Extend::Max,
            None,
            Some(1.0),
        )),
        "lapse_rate" | "lapse_rate_700_500" | "lrate" => Ok(make_style(
            colormaps::lapse_rate(),
            inclusive_levels(4.0, 10.0, 0.25),
            Extend::Both,
            None,
            Some(1.0),
        )),
        "uh" | "uhel" => Ok(make_style(
            colormaps::uh(),
            inclusive_levels(0.0, 210.0, 10.0),
            Extend::Max,
            None,
            Some(20.0),
        )),
        "vorticity" | "avo" | "pvo" | "wa" | "omega" => Ok(make_style(
            colormaps::relative_vorticity(128),
            inclusive_levels(-6.0e-4, 6.0e-4, 2.0e-5),
            Extend::Both,
            None,
            Some(1.0e-4),
        )),
        "height_anomaly" | "geopot_anomaly" | "geopt" => Ok(make_style(
            colormaps::geopot_anomaly(100),
            inclusive_levels(-600.0, 600.0, 25.0),
            Extend::Both,
            None,
            Some(100.0),
        )),
        "height" | "height_agl" => Ok(make_style(
            colormaps::geopot_anomaly(100),
            inclusive_levels(0.0, 16000.0, 1000.0),
            Extend::Both,
            None,
            Some(1000.0),
        )),
        "terrain" => Ok(make_style(
            colormaps::geopot_anomaly(100),
            inclusive_levels(0.0, 4200.0, 200.0),
            Extend::Max,
            None,
            Some(400.0),
        )),
        "slp" => Ok(make_style(
            colormaps::geopot_anomaly(100),
            inclusive_levels(980.0, 1042.0, 2.0),
            Extend::Both,
            None,
            Some(2.0),
        )),
        "pressure" => Ok(make_style(
            colormaps::geopot_anomaly(100),
            inclusive_levels(900.0, 1100.0, 5.0),
            Extend::Both,
            None,
            Some(5.0),
        )),
        "precip_in" | "precip" | "rain" => Ok(make_style(
            colormaps::precip_in(),
            precip_levels(),
            Extend::Max,
            None,
            Some(5.0),
        )),
        "sim_ir" | "ctt" => Ok(make_style(
            colormaps::sim_ir(),
            inclusive_levels(-80.0, 22.0, 2.0),
            Extend::Both,
            None,
            Some(10.0),
        )),
        "shaded_overlay" => Ok(make_style(
            colormaps::shaded_overlay(),
            vec![0.0, 1.0],
            Extend::Neither,
            None,
            None,
        )),
        "fosberg" => Ok(make_style(
            colormaps::temperature(180),
            inclusive_levels(0.0, 80.0, 5.0),
            Extend::Max,
            None,
            Some(5.0),
        )),
        "haines" => Ok(make_style(
            colormaps::temperature(180),
            inclusive_levels(2.0, 7.0, 1.0),
            Extend::Neither,
            None,
            Some(1.0),
        )),
        "hdw" => Ok(make_style(
            colormaps::temperature(180),
            inclusive_levels(0.0, 200.0, 10.0),
            Extend::Max,
            None,
            Some(10.0),
        )),
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

fn make_style(
    palette: Vec<Rgba<u8>>,
    levels: Vec<f64>,
    extend: Extend,
    mask_below: Option<f64>,
    tick_step: Option<f64>,
) -> StyleDefinition {
    StyleDefinition {
        palette,
        levels,
        extend,
        mask_below,
        tick_step,
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

fn precip_levels() -> Vec<f64> {
    let mut levels = Vec::new();
    levels.extend((0..10).map(|index| index as f64 * 0.1));
    levels.extend((2..10).map(|index| index as f64 * 0.5));
    levels.extend((5..25).map(|index| index as f64));
    levels.extend((25..55).step_by(5).map(|index| index as f64));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solar7_styles_resolve() {
        for style_id in [
            "dewpoint",
            "td",
            "rh2m",
            "sim_ir",
            "three_cape",
            "ehi",
            "lapse_rate_700_500",
            "uhel",
            "geopot_anomaly",
            "precip_in",
            "shaded_overlay",
        ] {
            let style = resolve_render_style(style_id, None, None, None)
                .unwrap_or_else(|err| panic!("{style_id} should resolve: {err}"));
            assert!(
                !style.colormap.levels.is_empty(),
                "{style_id} should have levels"
            );
        }
    }

    #[test]
    fn precip_levels_are_dense() {
        let levels = precip_levels();
        assert!(levels.len() > 40);
        assert_eq!(levels[0], 0.0);
        assert!(levels.last().copied().unwrap_or_default() >= 50.0);
    }
}
