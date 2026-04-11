use crate::text::{draw_text, draw_text_centered, draw_text_right, text_width};
use crate::{MapMarker, MapOverlaySpec, RenderedOverlay, color_for_value, palette_by_name};
use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Rgba, RgbaImage};
use shapefile::{Shape, ShapeReader};
use std::f64::consts::{FRAC_PI_4, PI};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use wx_types::{Field2D, ProjectionKind};

const EARTH_RADIUS_M: f64 = 6_370_000.0;
const DEG2RAD: f64 = PI / 180.0;

const CANVAS_WIDTH: u32 = 1600;
const CANVAS_HEIGHT: u32 = 1000;
const OUTER_PAD: u32 = 32;
const TITLE_HEIGHT: u32 = 54;
const FOOTER_HEIGHT: u32 = 42;
const COLORBAR_WIDTH: u32 = 28;
const COLORBAR_GAP: u32 = 28;
const COLORBAR_LABEL_SPACE: u32 = 110;

const PAGE_BG: Rgba<u8> = Rgba([239, 244, 249, 255]);
const MAP_BG: Rgba<u8> = Rgba([248, 251, 255, 255]);
const BORDER: Rgba<u8> = Rgba([90, 105, 120, 255]);
const COASTLINE: Rgba<u8> = Rgba([54, 77, 97, 255]);
const COUNTRY: Rgba<u8> = Rgba([102, 116, 130, 255]);
const STATE: Rgba<u8> = Rgba([150, 160, 172, 255]);
const GRATICULE: Rgba<u8> = Rgba([198, 209, 220, 255]);
const TITLE_TEXT: Rgba<u8> = Rgba([28, 39, 51, 255]);
const SUBTITLE_TEXT: Rgba<u8> = Rgba([76, 92, 110, 255]);
const MARKER_COLOR: Rgba<u8> = Rgba([206, 42, 42, 255]);

type LonLatLine = Vec<(f64, f64)>;

#[derive(Clone, Copy)]
struct PlotLayout {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
struct ValueScale {
    min: f32,
    max: f32,
    span: f32,
}

pub fn render_field_to_map_png(
    field: &Field2D,
    spec: &MapOverlaySpec,
    output_path: &Path,
) -> Result<RenderedOverlay> {
    validate_field_shape(field)?;

    let (value_min, value_max) = spec
        .value_range
        .or_else(|| field.finite_min_max())
        .context("field did not contain any finite values to render")?;
    let value_span = if (value_max - value_min).abs() < f32::EPSILON {
        1.0
    } else {
        value_max - value_min
    };
    let palette = palette_by_name(&spec.palette)?;
    let scale = ValueScale {
        min: value_min,
        max: value_max,
        span: value_span,
    };
    let layout = PlotLayout {
        x: OUTER_PAD,
        y: OUTER_PAD + TITLE_HEIGHT,
        width: CANVAS_WIDTH - OUTER_PAD * 2 - COLORBAR_WIDTH - COLORBAR_GAP - COLORBAR_LABEL_SPACE,
        height: CANVAS_HEIGHT - OUTER_PAD * 2 - TITLE_HEIGHT - FOOTER_HEIGHT,
    };

    let mut image: RgbaImage = ImageBuffer::from_pixel(CANVAS_WIDTH, CANVAS_HEIGHT, PAGE_BG);
    fill_rect(
        &mut image,
        layout.x,
        layout.y,
        layout.width,
        layout.height,
        MAP_BG,
    );

    let projected_grid = build_projected_grid(field, layout.width as f64 / layout.height as f64)?;
    rasterize_field(&mut image, field, &projected_grid, &palette, scale, layout);
    draw_graticule(&mut image, &projected_grid, layout);
    draw_basemap_features(&mut image, &projected_grid, layout);
    draw_marker_overlays(&mut image, &projected_grid, &spec.markers, layout);
    draw_rect_outline(
        &mut image,
        layout.x,
        layout.y,
        layout.width,
        layout.height,
        BORDER,
        2,
    );
    draw_titles(&mut image, field, spec, scale, layout);
    draw_colorbar(
        &mut image,
        &palette,
        scale,
        spec,
        PlotLayout {
            x: layout.x + layout.width + COLORBAR_GAP,
            y: layout.y + 36,
            width: COLORBAR_WIDTH,
            height: layout.height.saturating_sub(72),
        },
    );

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    image
        .save(output_path)
        .with_context(|| format!("failed to write {}", output_path.display()))?;

    Ok(RenderedOverlay {
        output_path: output_path.to_path_buf(),
        width: image.width(),
        height: image.height(),
        palette: spec.palette.clone(),
        value_min,
        value_max,
    })
}

fn validate_field_shape(field: &Field2D) -> Result<()> {
    let expected_len = field.expected_len();
    if field.values.len() != expected_len {
        bail!(
            "field value count {} does not match grid {}x{}",
            field.values.len(),
            field.grid.nx,
            field.grid.ny
        );
    }
    Ok(())
}

fn rasterize_field(
    image: &mut RgbaImage,
    field: &Field2D,
    projected_grid: &ProjectedGrid,
    palette: &[Rgba<u8>],
    scale: ValueScale,
    layout: PlotLayout,
) {
    for py in 0..layout.height {
        let y = projected_grid.extent.y_max
            - ((py as f64 + 0.5) / layout.height as f64)
                * (projected_grid.extent.y_max - projected_grid.extent.y_min);
        for px in 0..layout.width {
            let x = projected_grid.extent.x_min
                + ((px as f64 + 0.5) / layout.width as f64)
                    * (projected_grid.extent.x_max - projected_grid.extent.x_min);
            let i = ((x - projected_grid.x0) / projected_grid.x_step).round() as isize;
            let j = ((y - projected_grid.y0) / projected_grid.y_step).round() as isize;
            if i < 0 || j < 0 || i >= field.grid.nx as isize || j >= field.grid.ny as isize {
                continue;
            }

            let index = j as usize * field.grid.nx + i as usize;
            let value = field.values[index];
            if !value.is_finite() {
                continue;
            }

            let normalized = ((value - scale.min) / scale.span).clamp(0.0, 1.0);
            let mut color = color_for_value(palette, normalized);
            color.0[3] = 228;
            blend_pixel(image, (layout.x + px) as i32, (layout.y + py) as i32, color);
        }
    }
}

fn draw_titles(
    image: &mut RgbaImage,
    field: &Field2D,
    spec: &MapOverlaySpec,
    scale: ValueScale,
    layout: PlotLayout,
) {
    let title = spec.title.as_deref().unwrap_or(&field.metadata.parameter);
    let subtitle = spec.subtitle.clone().unwrap_or_else(|| {
        format!(
            "{} | {} | {} | valid {} | {:.2}..{:.2} {}",
            field.metadata.source.model.to_uppercase(),
            field.metadata.level.description,
            field.metadata.run.cycle.format("%Y-%m-%d %HZ"),
            field.metadata.valid.valid_time.format("%Y-%m-%d %HZ"),
            scale.min,
            scale.max,
            field.metadata.units
        )
    });
    draw_text(
        image,
        title,
        layout.x as i32,
        (layout.y - TITLE_HEIGHT + 6) as i32,
        TITLE_TEXT,
        2,
    );
    draw_text(
        image,
        &subtitle,
        layout.x as i32,
        (layout.y - TITLE_HEIGHT + 26) as i32,
        SUBTITLE_TEXT,
        1,
    );
    let footer = format!(
        "{} x {} | projection {}",
        field.grid.nx,
        field.grid.ny,
        projection_label(&field.grid.projection)
    );
    draw_text_right(
        image,
        &footer,
        (layout.x + layout.width) as i32,
        (layout.y + layout.height + 14) as i32,
        SUBTITLE_TEXT,
        1,
    );
}

fn draw_colorbar(
    image: &mut RgbaImage,
    palette: &[Rgba<u8>],
    scale: ValueScale,
    spec: &MapOverlaySpec,
    layout: PlotLayout,
) {
    for offset in 0..layout.height {
        let normalized = 1.0 - offset as f32 / (layout.height.max(1) - 1).max(1) as f32;
        let color = color_for_value(palette, normalized);
        for dx in 0..layout.width {
            image.put_pixel(layout.x + dx, layout.y + offset, color);
        }
    }
    draw_rect_outline(
        image,
        layout.x,
        layout.y,
        layout.width,
        layout.height,
        BORDER,
        1,
    );

    let label = spec
        .colorbar_label
        .clone()
        .unwrap_or_else(|| "value".to_string());
    draw_text_centered(
        image,
        &label,
        (layout.x + layout.width / 2) as i32,
        (layout.y.saturating_sub(18)) as i32,
        TITLE_TEXT,
        1,
    );

    for (tick, value) in [
        (0.0, scale.max),
        (0.25, scale.min + (scale.max - scale.min) * 0.75),
        (0.5, scale.min + (scale.max - scale.min) * 0.5),
        (0.75, scale.min + (scale.max - scale.min) * 0.25),
        (1.0, scale.min),
    ] {
        let ty = layout.y as f64 + tick * (layout.height.saturating_sub(1)) as f64;
        draw_line(
            image,
            layout.x as f64 + layout.width as f64 + 4.0,
            ty,
            layout.x as f64 + layout.width as f64 + 12.0,
            ty,
            BORDER,
            1,
        );
        draw_text(
            image,
            &format!("{value:.2}"),
            (layout.x + layout.width + 16) as i32,
            ty.round() as i32 - 4,
            SUBTITLE_TEXT,
            1,
        );
    }
}

fn draw_marker_overlays(
    image: &mut RgbaImage,
    projected_grid: &ProjectedGrid,
    markers: &[MapMarker],
    layout: PlotLayout,
) {
    for marker in markers {
        let marker_x = projected_grid.x0 + projected_grid.x_step * marker.grid_x as f64;
        let marker_y = projected_grid.y0 + projected_grid.y_step * marker.grid_y as f64;
        let Some((px, py)) =
            projected_grid
                .extent
                .to_pixel(marker_x, marker_y, layout.width, layout.height)
        else {
            continue;
        };
        let px = layout.x as f64 + px;
        let py = layout.y as f64 + py;
        draw_line(image, px - 8.0, py, px + 8.0, py, MARKER_COLOR, 2);
        draw_line(image, px, py - 8.0, px, py + 8.0, MARKER_COLOR, 2);
        if let Some(label) = &marker.label {
            let label_x = (px.round() as i32 + 12)
                .min((layout.x + layout.width).saturating_sub(text_width(label, 1) + 4) as i32);
            let label_y = (py.round() as i32 - 10).max(layout.y as i32 + 2);
            draw_text(image, label, label_x, label_y, MARKER_COLOR, 1);
        }
    }
}

fn draw_graticule(image: &mut RgbaImage, projected_grid: &ProjectedGrid, layout: PlotLayout) {
    let lat_bounds = projected_grid.lat_bounds();
    let lon_bounds = projected_grid.lon_bounds();
    let start_lat = ((lat_bounds.0 - 2.0) / 5.0).floor() as i32 * 5;
    let end_lat = ((lat_bounds.1 + 2.0) / 5.0).ceil() as i32 * 5;
    let start_lon = ((lon_bounds.0 - 2.0) / 5.0).floor() as i32 * 5;
    let end_lon = ((lon_bounds.1 + 2.0) / 5.0).ceil() as i32 * 5;

    for lat in (start_lat..=end_lat).step_by(5) {
        let mut points = Vec::new();
        for lon in start_lon..=end_lon {
            if let Some(point) = project_to_pixel(projected_grid, lat as f64, lon as f64, layout) {
                points.push(point);
            }
        }
        draw_polyline(image, &points, GRATICULE, 1);
    }

    for lon in (start_lon..=end_lon).step_by(5) {
        let mut points = Vec::new();
        for lat in start_lat..=end_lat {
            if let Some(point) = project_to_pixel(projected_grid, lat as f64, lon as f64, layout) {
                points.push(point);
            }
        }
        draw_polyline(image, &points, GRATICULE, 1);
    }
}

fn draw_basemap_features(
    image: &mut RgbaImage,
    projected_grid: &ProjectedGrid,
    layout: PlotLayout,
) {
    for layer in load_basemap_layers() {
        for line in &layer.lines {
            for segment in line.windows(2) {
                let Some((x0, y0)) =
                    project_to_pixel(projected_grid, segment[0].1, segment[0].0, layout)
                else {
                    continue;
                };
                let Some((x1, y1)) =
                    project_to_pixel(projected_grid, segment[1].1, segment[1].0, layout)
                else {
                    continue;
                };
                draw_line(image, x0, y0, x1, y1, layer.color, layer.width);
            }
        }
    }
}

fn project_to_pixel(
    projected_grid: &ProjectedGrid,
    lat: f64,
    lon: f64,
    layout: PlotLayout,
) -> Option<(f64, f64)> {
    let (x, y) = projected_grid.projector.project(lat, lon);
    let (px, py) = projected_grid
        .extent
        .to_pixel(x, y, layout.width, layout.height)?;
    Some((layout.x as f64 + px, layout.y as f64 + py))
}

fn load_basemap_layers() -> &'static [BasemapLayer] {
    static CACHE: OnceLock<Vec<BasemapLayer>> = OnceLock::new();
    CACHE.get_or_init(|| {
        [
            ("ne_110m_coastline.shp", COASTLINE, 2),
            ("ne_110m_admin_0_boundary_lines_land.shp", COUNTRY, 1),
            ("ne_110m_admin_1_states_provinces_lines.shp", STATE, 1),
        ]
        .into_iter()
        .filter_map(|(name, color, width)| {
            let path = basemap_asset_root().join(name);
            load_lines_from_shapefile(&path)
                .ok()
                .map(|lines| BasemapLayer {
                    lines,
                    color,
                    width,
                })
        })
        .collect()
    })
}

fn basemap_asset_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace layout is stable")
        .join("assets")
        .join("basemap")
        .join("natural_earth_110m")
}

fn load_lines_from_shapefile(path: &Path) -> Result<Vec<LonLatLine>> {
    let mut reader = ShapeReader::from_path(path)
        .with_context(|| format!("failed to open basemap shapefile {}", path.display()))?;
    let mut lines = Vec::new();

    for shape in reader.iter_shapes() {
        match shape? {
            Shape::Polyline(polyline) => {
                for part in polyline.parts() {
                    let points: LonLatLine = part.iter().map(|point| (point.x, point.y)).collect();
                    if points.len() >= 2 {
                        lines.push(points);
                    }
                }
            }
            Shape::Polygon(polygon) => {
                for ring in polygon.rings() {
                    let points: LonLatLine = ring
                        .points()
                        .iter()
                        .map(|point| (point.x, point.y))
                        .collect();
                    if points.len() >= 2 {
                        lines.push(points);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(lines)
}

fn build_projected_grid(field: &Field2D, target_ratio: f64) -> Result<ProjectedGrid> {
    let projector = Projector::from_field(field)?;
    let (x0, y0) = projector.project(
        field.grid.coordinates.lat1,
        normalize_lon(field.grid.coordinates.lon1),
    );
    let (x_last, y_last) = projector.project(
        field.grid.coordinates.lat2,
        normalize_lon(field.grid.coordinates.lon2),
    );

    let x_sign = if (x_last - x0).abs() < 1.0 {
        1.0
    } else {
        (x_last - x0).signum()
    };
    let y_sign = if (y_last - y0).abs() < 1.0 {
        1.0
    } else {
        (y_last - y0).signum()
    };
    let x_step = field.grid.coordinates.dx * x_sign;
    let y_step = field.grid.coordinates.dy * y_sign;
    let x_end = x0 + x_step * (field.grid.nx.saturating_sub(1) as f64);
    let y_end = y0 + y_step * (field.grid.ny.saturating_sub(1) as f64);
    let extent = MapExtent::from_bounds(
        x0.min(x_end),
        x0.max(x_end),
        y0.min(y_end),
        y0.max(y_end),
        target_ratio,
    );

    Ok(ProjectedGrid {
        projector,
        extent,
        x0,
        y0,
        x_step,
        y_step,
        lat1: field.grid.coordinates.lat1,
        lon1: normalize_lon(field.grid.coordinates.lon1),
        lat2: field.grid.coordinates.lat2,
        lon2: normalize_lon(field.grid.coordinates.lon2),
    })
}

fn projection_label(kind: &ProjectionKind) -> &'static str {
    match kind {
        ProjectionKind::LatitudeLongitude => "lat/lon",
        ProjectionKind::LambertConformal { .. } => "lambert conformal",
        ProjectionKind::Mercator { .. } => "mercator",
        ProjectionKind::PolarStereographic { .. } => "polar stereographic",
        ProjectionKind::Unknown { .. } => "unknown",
    }
}

fn normalize_lon(mut lon: f64) -> f64 {
    while lon > 180.0 {
        lon -= 360.0;
    }
    while lon < -180.0 {
        lon += 360.0;
    }
    lon
}

#[derive(Clone)]
struct BasemapLayer {
    lines: Vec<LonLatLine>,
    color: Rgba<u8>,
    width: u32,
}

#[derive(Clone)]
struct ProjectedGrid {
    projector: Projector,
    extent: MapExtent,
    x0: f64,
    y0: f64,
    x_step: f64,
    y_step: f64,
    lat1: f64,
    lon1: f64,
    lat2: f64,
    lon2: f64,
}

impl ProjectedGrid {
    fn lat_bounds(&self) -> (f64, f64) {
        (self.lat1.min(self.lat2), self.lat1.max(self.lat2))
    }

    fn lon_bounds(&self) -> (f64, f64) {
        (self.lon1.min(self.lon2), self.lon1.max(self.lon2))
    }
}

#[derive(Clone)]
struct MapExtent {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
}

impl MapExtent {
    fn from_bounds(x_min: f64, x_max: f64, y_min: f64, y_max: f64, target_ratio: f64) -> Self {
        let data_width = x_max - x_min;
        let data_height = y_max - y_min;
        let data_ratio = data_width / data_height.max(1.0);

        if data_ratio > target_ratio {
            let new_height = data_width / target_ratio;
            let pad = (new_height - data_height) / 2.0;
            Self {
                x_min,
                x_max,
                y_min: y_min - pad,
                y_max: y_max + pad,
            }
        } else {
            let new_width = data_height * target_ratio;
            let pad = (new_width - data_width) / 2.0;
            Self {
                x_min: x_min - pad,
                x_max: x_max + pad,
                y_min,
                y_max,
            }
        }
    }

    fn to_pixel(&self, x: f64, y: f64, width: u32, height: u32) -> Option<(f64, f64)> {
        let rx = (x - self.x_min) / (self.x_max - self.x_min);
        let ry = 1.0 - (y - self.y_min) / (self.y_max - self.y_min);
        if !(-0.2..=1.2).contains(&rx) || !(-0.2..=1.2).contains(&ry) {
            return None;
        }
        Some((
            rx * (width.saturating_sub(1)) as f64,
            ry * (height.saturating_sub(1)) as f64,
        ))
    }
}

#[derive(Clone)]
enum Projector {
    LatLon,
    Lambert(LambertConformal),
    Mercator(MercatorProjection),
    PolarStereo(PolarStereographicProjection),
}

impl Projector {
    fn from_field(field: &Field2D) -> Result<Self> {
        match &field.grid.projection {
            ProjectionKind::LatitudeLongitude => Ok(Self::LatLon),
            ProjectionKind::LambertConformal {
                latin1,
                latin2,
                lov,
            } => Ok(Self::Lambert(LambertConformal::new(
                *latin1,
                *latin2,
                *lov,
                (field.grid.coordinates.lat1 + field.grid.coordinates.lat2) / 2.0,
            ))),
            ProjectionKind::Mercator { lad } => Ok(Self::Mercator(MercatorProjection::new(
                (field.grid.coordinates.lon1 + field.grid.coordinates.lon2) / 2.0,
                *lad,
            ))),
            ProjectionKind::PolarStereographic { lad, lov } => Ok(Self::PolarStereo(
                PolarStereographicProjection::new(*lad, *lov),
            )),
            ProjectionKind::Unknown { template } => {
                bail!("unsupported projection template {template} for basemap rendering")
            }
        }
    }

    fn project(&self, lat: f64, lon: f64) -> (f64, f64) {
        match self {
            Self::LatLon => (normalize_lon(lon), lat),
            Self::Lambert(projection) => projection.project(lat, lon),
            Self::Mercator(projection) => projection.project(lat, lon),
            Self::PolarStereo(projection) => projection.project(lat, lon),
        }
    }
}

#[derive(Clone)]
struct LambertConformal {
    n: f64,
    f: f64,
    rho0: f64,
    lambda0: f64,
}

impl LambertConformal {
    fn new(truelat1: f64, truelat2: f64, stand_lon: f64, ref_lat: f64) -> Self {
        let phi1 = truelat1 * DEG2RAD;
        let phi2 = truelat2 * DEG2RAD;
        let phi0 = ref_lat * DEG2RAD;
        let lambda0 = stand_lon * DEG2RAD;

        let n = if (truelat1 - truelat2).abs() < 1e-10 {
            phi1.sin()
        } else {
            let numerator = phi1.cos().ln() - phi2.cos().ln();
            let denominator =
                (FRAC_PI_4 + phi2 / 2.0).tan().ln() - (FRAC_PI_4 + phi1 / 2.0).tan().ln();
            numerator / denominator
        };
        let f = phi1.cos() * (FRAC_PI_4 + phi1 / 2.0).tan().powf(n) / n;
        let rho0 = EARTH_RADIUS_M * f / (FRAC_PI_4 + phi0 / 2.0).tan().powf(n);

        Self {
            n,
            f,
            rho0,
            lambda0,
        }
    }

    fn project(&self, lat: f64, lon: f64) -> (f64, f64) {
        let phi = lat * DEG2RAD;
        let lambda = normalize_lon(lon) * DEG2RAD;
        let rho = EARTH_RADIUS_M * self.f / (FRAC_PI_4 + phi / 2.0).tan().powf(self.n);
        let theta = self.n * (lambda - self.lambda0);
        let x = rho * theta.sin();
        let y = self.rho0 - rho * theta.cos();
        (x, y)
    }
}

#[derive(Clone)]
struct MercatorProjection {
    lambda0: f64,
}

impl MercatorProjection {
    fn new(central_lon: f64, _lad: f64) -> Self {
        Self {
            lambda0: normalize_lon(central_lon) * DEG2RAD,
        }
    }

    fn project(&self, lat: f64, lon: f64) -> (f64, f64) {
        let phi = lat.clamp(-85.0, 85.0) * DEG2RAD;
        let lambda = normalize_lon(lon) * DEG2RAD;
        (
            EARTH_RADIUS_M * (lambda - self.lambda0),
            EARTH_RADIUS_M * (FRAC_PI_4 + phi / 2.0).tan().ln(),
        )
    }
}

#[derive(Clone)]
struct PolarStereographicProjection {
    lambda0: f64,
    north: bool,
}

impl PolarStereographicProjection {
    fn new(lad: f64, lov: f64) -> Self {
        Self {
            lambda0: normalize_lon(lov) * DEG2RAD,
            north: lad >= 0.0,
        }
    }

    fn project(&self, lat: f64, lon: f64) -> (f64, f64) {
        let phi = lat * DEG2RAD;
        let lambda = normalize_lon(lon) * DEG2RAD;
        let theta = lambda - self.lambda0;
        let rho = if self.north {
            2.0 * EARTH_RADIUS_M * (FRAC_PI_4 - phi / 2.0).tan()
        } else {
            2.0 * EARTH_RADIUS_M * (FRAC_PI_4 + phi / 2.0).tan()
        };
        (rho * theta.sin(), -rho * theta.cos())
    }
}

fn fill_rect(image: &mut RgbaImage, x: u32, y: u32, width: u32, height: u32, color: Rgba<u8>) {
    for py in y..(y + height).min(image.height()) {
        for px in x..(x + width).min(image.width()) {
            image.put_pixel(px, py, color);
        }
    }
}

fn draw_rect_outline(
    image: &mut RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: Rgba<u8>,
    line_width: u32,
) {
    for offset in 0..line_width {
        let ox = x.saturating_add(offset);
        let oy = y.saturating_add(offset);
        let ow = width.saturating_sub(offset * 2);
        let oh = height.saturating_sub(offset * 2);
        if ow == 0 || oh == 0 {
            continue;
        }
        draw_line(
            image,
            ox as f64,
            oy as f64,
            (ox + ow - 1) as f64,
            oy as f64,
            color,
            1,
        );
        draw_line(
            image,
            ox as f64,
            (oy + oh - 1) as f64,
            (ox + ow - 1) as f64,
            (oy + oh - 1) as f64,
            color,
            1,
        );
        draw_line(
            image,
            ox as f64,
            oy as f64,
            ox as f64,
            (oy + oh - 1) as f64,
            color,
            1,
        );
        draw_line(
            image,
            (ox + ow - 1) as f64,
            oy as f64,
            (ox + ow - 1) as f64,
            (oy + oh - 1) as f64,
            color,
            1,
        );
    }
}

fn draw_polyline(image: &mut RgbaImage, points: &[(f64, f64)], color: Rgba<u8>, width: u32) {
    if points.len() < 2 {
        return;
    }
    for segment in points.windows(2) {
        draw_line(
            image,
            segment[0].0,
            segment[0].1,
            segment[1].0,
            segment[1].1,
            color,
            width,
        );
    }
}

fn draw_line(
    image: &mut RgbaImage,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    color: Rgba<u8>,
    width: u32,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let steps = dx.abs().max(dy.abs()).ceil() as usize;
    if steps == 0 {
        blend_pixel(image, x0.round() as i32, y0.round() as i32, color);
        return;
    }
    let radius = (width as i32).saturating_sub(1) / 2;
    for step in 0..=steps {
        let fraction = step as f64 / steps as f64;
        let x = x0 + dx * fraction;
        let y = y0 + dy * fraction;
        for oy in -radius..=radius {
            for ox in -radius..=radius {
                blend_pixel(image, x.round() as i32 + ox, y.round() as i32 + oy, color);
            }
        }
    }
}

fn blend_pixel(image: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x < 0 || y < 0 || (x as u32) >= image.width() || (y as u32) >= image.height() {
        return;
    }
    if color.0[3] == 255 {
        image.put_pixel(x as u32, y as u32, color);
        return;
    }
    if color.0[3] == 0 {
        return;
    }

    let dst = image.get_pixel(x as u32, y as u32).0;
    let alpha = color.0[3] as f64 / 255.0;
    let inv = 1.0 - alpha;
    image.put_pixel(
        x as u32,
        y as u32,
        Rgba([
            (color.0[0] as f64 * alpha + dst[0] as f64 * inv).round() as u8,
            (color.0[1] as f64 * alpha + dst[1] as f64 * inv).round() as u8,
            (color.0[2] as f64 * alpha + dst[2] as f64 * inv).round() as u8,
            255,
        ]),
    );
}
