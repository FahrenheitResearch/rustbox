use crate::style::{RenderStyle, format_tick, resolve_render_style};
use crate::text::{draw_text, draw_text_centered, draw_text_right, text_width};
use crate::{MapMarker, MapOverlaySpec, RenderedOverlay};
use anyhow::{Context, Result, bail};
use image::{ImageBuffer, Rgba, RgbaImage};
use shapefile::{Shape, ShapeReader};
use std::f64::consts::{FRAC_PI_4, PI};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use wx_types::{Field2D, ProjectionKind};

const EARTH_RADIUS_M: f64 = 6_371_229.0;
const DEG2RAD: f64 = PI / 180.0;
const RAD2DEG: f64 = 180.0 / PI;

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
const GRATICULE: Rgba<u8> = Rgba([206, 215, 224, 255]);
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
    let scale = ValueScale {
        min: value_min,
        max: value_max,
    };
    let style = resolve_render_style(&spec.palette, spec.value_range)?;
    let layout = PlotLayout {
        x: OUTER_PAD,
        y: OUTER_PAD + TITLE_HEIGHT,
        width: CANVAS_WIDTH - OUTER_PAD * 2 - COLORBAR_WIDTH - COLORBAR_GAP - COLORBAR_LABEL_SPACE,
        height: CANVAS_HEIGHT - OUTER_PAD * 2 - TITLE_HEIGHT - FOOTER_HEIGHT,
    };

    let model_grid = build_model_grid(field, layout.width as f64 / layout.height as f64)?;
    let mut image: RgbaImage = ImageBuffer::from_pixel(CANVAS_WIDTH, CANVAS_HEIGHT, PAGE_BG);
    fill_rect(
        &mut image,
        layout.x,
        layout.y,
        layout.width,
        layout.height,
        MAP_BG,
    );

    rasterize_field(&mut image, field, &model_grid, &style, layout);
    draw_graticule(&mut image, &model_grid.geo_extent, layout);
    draw_basemap_features(&mut image, &model_grid.geo_extent, layout);
    draw_marker_overlays(&mut image, &model_grid, &spec.markers, layout);
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
        &style,
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
        palette: style.id,
        value_min,
        value_max,
    })
}

fn validate_field_shape(field: &Field2D) -> Result<()> {
    if field.values.len() != field.expected_len() {
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
    model_grid: &ModelGrid,
    style: &RenderStyle,
    layout: PlotLayout,
) {
    for py in 0..layout.height {
        let lat = model_grid.geo_extent.lat_max
            - ((py as f64 + 0.5) / layout.height as f64)
                * (model_grid.geo_extent.lat_max - model_grid.geo_extent.lat_min);
        for px in 0..layout.width {
            let lon = model_grid.geo_extent.lon_min
                + ((px as f64 + 0.5) / layout.width as f64)
                    * (model_grid.geo_extent.lon_max - model_grid.geo_extent.lon_min);
            let Some((grid_i, grid_j)) = model_grid.projector.latlon_to_grid(lat, lon) else {
                continue;
            };
            let i = grid_i.round() as isize;
            let j = grid_j.round() as isize;
            if i < 0 || j < 0 || i >= field.grid.nx as isize || j >= field.grid.ny as isize {
                continue;
            }

            let value = field.values[j as usize * field.grid.nx + i as usize];
            if !value.is_finite() {
                continue;
            }

            let mut color = style.color_for_value(value as f64);
            if color.0[3] == 0 {
                continue;
            }
            color.0[3] = 232;
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
    draw_text_centered(
        image,
        title,
        (layout.x + layout.width / 2) as i32,
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
    style: &RenderStyle,
    spec: &MapOverlaySpec,
    layout: PlotLayout,
) {
    let Some((range_min, range_max)) = style.colormap.range() else {
        return;
    };
    let ticks = style.tick_values();

    for offset in 0..layout.height {
        let normalized = 1.0 - offset as f64 / (layout.height.max(1) - 1).max(1) as f64;
        let value = range_min + (range_max - range_min) * normalized;
        let color = style.color_for_value(value);
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
        layout.y.saturating_sub(18) as i32,
        TITLE_TEXT,
        1,
    );

    for value in ticks {
        let ty = layout.y as f64
            + (1.0 - ((value - range_min) / (range_max - range_min).max(f64::EPSILON)))
                * (layout.height.saturating_sub(1)) as f64;
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
            &format_tick(value),
            (layout.x + layout.width + 16) as i32,
            ty.round() as i32 - 4,
            SUBTITLE_TEXT,
            1,
        );
    }
}

fn draw_marker_overlays(
    image: &mut RgbaImage,
    model_grid: &ModelGrid,
    markers: &[MapMarker],
    layout: PlotLayout,
) {
    for marker in markers {
        let Some((lat, lon)) = model_grid.grid_to_latlon(marker.grid_x, marker.grid_y) else {
            continue;
        };
        let Some((px, py)) =
            model_grid
                .geo_extent
                .pixel_coords(lon, lat, layout.width, layout.height)
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

fn draw_graticule(image: &mut RgbaImage, extent: &GeoExtent, layout: PlotLayout) {
    let start_lat = ((extent.lat_min - 2.0) / 5.0).floor() as i32 * 5;
    let end_lat = ((extent.lat_max + 2.0) / 5.0).ceil() as i32 * 5;
    let start_lon = ((extent.lon_min - 2.0) / 5.0).floor() as i32 * 5;
    let end_lon = ((extent.lon_max + 2.0) / 5.0).ceil() as i32 * 5;

    for lat in (start_lat..=end_lat).step_by(5) {
        if let (Some((x0, y0)), Some((x1, y1))) = (
            extent.pixel_coords(start_lon as f64, lat as f64, layout.width, layout.height),
            extent.pixel_coords(end_lon as f64, lat as f64, layout.width, layout.height),
        ) {
            draw_line(
                image,
                layout.x as f64 + x0,
                layout.y as f64 + y0,
                layout.x as f64 + x1,
                layout.y as f64 + y1,
                GRATICULE,
                1,
            );
        }
    }

    for lon in (start_lon..=end_lon).step_by(5) {
        if let (Some((x0, y0)), Some((x1, y1))) = (
            extent.pixel_coords(lon as f64, start_lat as f64, layout.width, layout.height),
            extent.pixel_coords(lon as f64, end_lat as f64, layout.width, layout.height),
        ) {
            draw_line(
                image,
                layout.x as f64 + x0,
                layout.y as f64 + y0,
                layout.x as f64 + x1,
                layout.y as f64 + y1,
                GRATICULE,
                1,
            );
        }
    }
}

fn draw_basemap_features(image: &mut RgbaImage, extent: &GeoExtent, layout: PlotLayout) {
    for layer in load_basemap_layers() {
        for line in &layer.lines {
            let mut projected = Vec::with_capacity(line.len());
            for (lon, lat) in line {
                if let Some((px, py)) = extent.pixel_coords(*lon, *lat, layout.width, layout.height)
                {
                    projected.push((layout.x as f64 + px, layout.y as f64 + py));
                }
            }
            draw_polyline(image, &projected, layer.color, layer.width);
        }
    }
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

fn build_model_grid(field: &Field2D, target_ratio: f64) -> Result<ModelGrid> {
    let projector = Projector::from_field(field)?;
    let nx = field.grid.nx.saturating_sub(1) as f64;
    let ny = field.grid.ny.saturating_sub(1) as f64;
    let corners = [
        projector.grid_to_latlon(0.0, 0.0),
        projector.grid_to_latlon(nx, 0.0),
        projector.grid_to_latlon(0.0, ny),
        projector.grid_to_latlon(nx, ny),
    ];
    let mut lat_min = f64::INFINITY;
    let mut lat_max = f64::NEG_INFINITY;
    let mut lon_min = f64::INFINITY;
    let mut lon_max = f64::NEG_INFINITY;
    for corner in corners {
        let Some((lat, lon)) = corner else {
            continue;
        };
        lat_min = lat_min.min(lat);
        lat_max = lat_max.max(lat);
        lon_min = lon_min.min(lon);
        lon_max = lon_max.max(lon);
    }
    if !lat_min.is_finite() || !lon_min.is_finite() {
        bail!("failed to derive geographic bounds for projected field");
    }

    Ok(ModelGrid {
        projector,
        geo_extent: GeoExtent::from_bounds(lon_min, lon_max, lat_min, lat_max, target_ratio),
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
struct ModelGrid {
    projector: Projector,
    geo_extent: GeoExtent,
}

impl ModelGrid {
    fn grid_to_latlon(&self, grid_x: usize, grid_y: usize) -> Option<(f64, f64)> {
        self.projector.grid_to_latlon(grid_x as f64, grid_y as f64)
    }
}

#[derive(Clone, Copy)]
struct GeoExtent {
    lon_min: f64,
    lon_max: f64,
    lat_min: f64,
    lat_max: f64,
}

impl GeoExtent {
    fn from_bounds(
        lon_min: f64,
        lon_max: f64,
        lat_min: f64,
        lat_max: f64,
        target_ratio: f64,
    ) -> Self {
        let mid_lat = ((lat_min + lat_max) * 0.5)
            .to_radians()
            .cos()
            .abs()
            .max(0.1);
        let data_width = (lon_max - lon_min) * mid_lat;
        let data_height = lat_max - lat_min;
        let data_ratio = data_width / data_height.max(1e-6);

        if data_ratio > target_ratio {
            let new_height = data_width / target_ratio;
            let pad = (new_height - data_height) * 0.5;
            Self {
                lon_min,
                lon_max,
                lat_min: lat_min - pad,
                lat_max: lat_max + pad,
            }
        } else {
            let new_width = data_height * target_ratio;
            let pad = (new_width - data_width) * 0.5 / mid_lat;
            Self {
                lon_min: lon_min - pad,
                lon_max: lon_max + pad,
                lat_min,
                lat_max,
            }
        }
    }

    fn pixel_coords(&self, lon: f64, lat: f64, width: u32, height: u32) -> Option<(f64, f64)> {
        let rx = (lon - self.lon_min) / (self.lon_max - self.lon_min);
        let ry = 1.0 - (lat - self.lat_min) / (self.lat_max - self.lat_min);
        if !(-0.05..=1.05).contains(&rx) || !(-0.05..=1.05).contains(&ry) {
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
    LatLon(LatLonProjection),
    Lambert(HrrrLambertProjection),
    Mercator(MercatorProjection),
    PolarStereo(PolarStereographicProjection),
}

impl Projector {
    fn from_field(field: &Field2D) -> Result<Self> {
        match &field.grid.projection {
            ProjectionKind::LatitudeLongitude => Ok(Self::LatLon(LatLonProjection::new(
                field.grid.coordinates.lat1,
                normalize_lon(field.grid.coordinates.lon1),
                field.grid.coordinates.lat2,
                normalize_lon(field.grid.coordinates.lon2),
                field.grid.nx,
                field.grid.ny,
            ))),
            ProjectionKind::LambertConformal {
                latin1,
                latin2,
                lov,
            } => Ok(Self::Lambert(HrrrLambertProjection::new(
                *latin1,
                *latin2,
                *lov,
                field.grid.coordinates.lat1,
                normalize_lon(field.grid.coordinates.lon1),
                field.grid.coordinates.dx,
                field.grid.coordinates.dy,
            ))),
            ProjectionKind::Mercator { lad } => Ok(Self::Mercator(MercatorProjection::new(
                (field.grid.coordinates.lon1 + field.grid.coordinates.lon2) * 0.5,
                *lad,
                field.grid.coordinates.lat1,
                normalize_lon(field.grid.coordinates.lon1),
                field.grid.coordinates.dx,
                field.grid.coordinates.dy,
            ))),
            ProjectionKind::PolarStereographic { lad, lov } => {
                Ok(Self::PolarStereo(PolarStereographicProjection::new(
                    *lad,
                    *lov,
                    field.grid.coordinates.lat1,
                    normalize_lon(field.grid.coordinates.lon1),
                    field.grid.coordinates.dx,
                    field.grid.coordinates.dy,
                )))
            }
            ProjectionKind::Unknown { template } => {
                bail!("unsupported projection template {template} for basemap rendering")
            }
        }
    }

    fn latlon_to_grid(&self, lat: f64, lon: f64) -> Option<(f64, f64)> {
        match self {
            Self::LatLon(projection) => projection.latlon_to_grid(lat, lon),
            Self::Lambert(projection) => Some(projection.latlon_to_grid(lat, lon)),
            Self::Mercator(projection) => Some(projection.latlon_to_grid(lat, lon)),
            Self::PolarStereo(projection) => Some(projection.latlon_to_grid(lat, lon)),
        }
    }

    fn grid_to_latlon(&self, grid_x: f64, grid_y: f64) -> Option<(f64, f64)> {
        match self {
            Self::LatLon(projection) => projection.grid_to_latlon(grid_x, grid_y),
            Self::Lambert(projection) => Some(projection.grid_to_latlon(grid_x, grid_y)),
            Self::Mercator(projection) => Some(projection.grid_to_latlon(grid_x, grid_y)),
            Self::PolarStereo(projection) => Some(projection.grid_to_latlon(grid_x, grid_y)),
        }
    }
}

#[derive(Clone)]
struct HrrrLambertProjection {
    n: f64,
    f_val: f64,
    lov: f64,
    rho1: f64,
    theta1: f64,
    dx: f64,
    dy: f64,
}

impl HrrrLambertProjection {
    fn new(
        truelat1: f64,
        truelat2: f64,
        stand_lon: f64,
        lat1: f64,
        lon1: f64,
        dx: f64,
        dy: f64,
    ) -> Self {
        let phi1 = truelat1 * DEG2RAD;
        let phi2 = truelat2 * DEG2RAD;
        let la1 = lat1 * DEG2RAD;
        let lo1 = normalize_lon(lon1) * DEG2RAD;
        let lov = normalize_lon(stand_lon) * DEG2RAD;

        let n = if (truelat1 - truelat2).abs() < 1e-10 {
            phi1.sin()
        } else {
            let ln_ratio =
                (FRAC_PI_4 + phi2 / 2.0).tan().ln() - (FRAC_PI_4 + phi1 / 2.0).tan().ln();
            (phi1.cos().ln() - phi2.cos().ln()) / ln_ratio
        };
        let f_val = phi1.cos() * (FRAC_PI_4 + phi1 / 2.0).tan().powf(n) / n;
        let rho1 = EARTH_RADIUS_M * f_val / (FRAC_PI_4 + la1 / 2.0).tan().powf(n);
        let theta1 = n * (lo1 - lov);

        Self {
            n,
            f_val,
            lov,
            rho1,
            theta1,
            dx,
            dy,
        }
    }

    fn latlon_to_grid(&self, lat: f64, lon: f64) -> (f64, f64) {
        let phi = lat * DEG2RAD;
        let lambda = normalize_lon(lon) * DEG2RAD;
        let rho = EARTH_RADIUS_M * self.f_val / (FRAC_PI_4 + phi / 2.0).tan().powf(self.n);
        let theta = self.n * (lambda - self.lov);
        let x = rho * theta.sin() - self.rho1 * self.theta1.sin();
        let y = self.rho1 * self.theta1.cos() - rho * theta.cos();
        (x / self.dx, y / self.dy)
    }

    fn grid_to_latlon(&self, grid_x: f64, grid_y: f64) -> (f64, f64) {
        let x = self.rho1 * self.theta1.sin() + grid_x * self.dx;
        let y = self.rho1 * self.theta1.cos() - grid_y * self.dy;
        let rho = (x * x + y * y).sqrt() * self.n.signum();
        let theta = x.atan2(y);
        let lat = (2.0 * ((EARTH_RADIUS_M * self.f_val / rho).powf(1.0 / self.n)).atan()
            - PI / 2.0)
            * RAD2DEG;
        let lon = normalize_lon((self.lov + theta / self.n) * RAD2DEG);
        (lat, lon)
    }
}

#[derive(Clone)]
struct LatLonProjection {
    lat1: f64,
    lon1: f64,
    lat2: f64,
    lon2: f64,
    nx: usize,
    ny: usize,
}

impl LatLonProjection {
    fn new(lat1: f64, lon1: f64, lat2: f64, lon2: f64, nx: usize, ny: usize) -> Self {
        Self {
            lat1,
            lon1,
            lat2,
            lon2,
            nx,
            ny,
        }
    }

    fn latlon_to_grid(&self, lat: f64, lon: f64) -> Option<(f64, f64)> {
        let dlon = self.lon2 - self.lon1;
        let dlat = self.lat2 - self.lat1;
        if dlon.abs() < 1e-9 || dlat.abs() < 1e-9 {
            return None;
        }
        let i = (normalize_lon(lon) - self.lon1) * (self.nx.saturating_sub(1)) as f64 / dlon;
        let j = (lat - self.lat1) * (self.ny.saturating_sub(1)) as f64 / dlat;
        Some((i, j))
    }

    fn grid_to_latlon(&self, grid_x: f64, grid_y: f64) -> Option<(f64, f64)> {
        let lon = self.lon1
            + grid_x * (self.lon2 - self.lon1) / (self.nx.saturating_sub(1)).max(1) as f64;
        let lat = self.lat1
            + grid_y * (self.lat2 - self.lat1) / (self.ny.saturating_sub(1)).max(1) as f64;
        Some((lat, normalize_lon(lon)))
    }
}

#[derive(Clone)]
struct MercatorProjection {
    lambda0: f64,
    x0: f64,
    y0: f64,
    dx: f64,
    dy: f64,
}

impl MercatorProjection {
    fn new(central_lon: f64, _lad: f64, lat1: f64, lon1: f64, dx: f64, dy: f64) -> Self {
        let mut projection = Self {
            lambda0: normalize_lon(central_lon) * DEG2RAD,
            x0: 0.0,
            y0: 0.0,
            dx,
            dy,
        };
        let (x0, y0) = projection.project_xy(lat1, lon1);
        projection.x0 = x0;
        projection.y0 = y0;
        projection
    }

    fn project_xy(&self, lat: f64, lon: f64) -> (f64, f64) {
        let phi = lat.clamp(-85.0, 85.0) * DEG2RAD;
        let lambda = normalize_lon(lon) * DEG2RAD;
        (
            EARTH_RADIUS_M * (lambda - self.lambda0),
            EARTH_RADIUS_M * (FRAC_PI_4 + phi / 2.0).tan().ln(),
        )
    }

    fn latlon_to_grid(&self, lat: f64, lon: f64) -> (f64, f64) {
        let (x, y) = self.project_xy(lat, lon);
        ((x - self.x0) / self.dx, (y - self.y0) / self.dy)
    }

    fn grid_to_latlon(&self, grid_x: f64, grid_y: f64) -> (f64, f64) {
        self.inverse_xy(self.x0 + grid_x * self.dx, self.y0 + grid_y * self.dy)
    }

    fn inverse_xy(&self, x: f64, y: f64) -> (f64, f64) {
        let lambda = x / EARTH_RADIUS_M + self.lambda0;
        let phi = 2.0 * (y / EARTH_RADIUS_M).exp().atan() - PI * 0.5;
        (phi * RAD2DEG, normalize_lon(lambda * RAD2DEG))
    }
}

#[derive(Clone)]
struct PolarStereographicProjection {
    lambda0: f64,
    north: bool,
    x0: f64,
    y0: f64,
    dx: f64,
    dy: f64,
}

impl PolarStereographicProjection {
    fn new(lad: f64, lov: f64, lat1: f64, lon1: f64, dx: f64, dy: f64) -> Self {
        let mut projection = Self {
            lambda0: normalize_lon(lov) * DEG2RAD,
            north: lad >= 0.0,
            x0: 0.0,
            y0: 0.0,
            dx,
            dy,
        };
        let (x0, y0) = projection.project_xy(lat1, lon1);
        projection.x0 = x0;
        projection.y0 = y0;
        projection
    }

    fn project_xy(&self, lat: f64, lon: f64) -> (f64, f64) {
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

    fn latlon_to_grid(&self, lat: f64, lon: f64) -> (f64, f64) {
        let (x, y) = self.project_xy(lat, lon);
        ((x - self.x0) / self.dx, (y - self.y0) / self.dy)
    }

    fn grid_to_latlon(&self, grid_x: f64, grid_y: f64) -> (f64, f64) {
        self.inverse_xy(self.x0 + grid_x * self.dx, self.y0 + grid_y * self.dy)
    }

    fn inverse_xy(&self, x: f64, y: f64) -> (f64, f64) {
        let rho = (x * x + y * y).sqrt();
        let theta = x.atan2(-y);
        if self.north {
            let phi = PI * 0.5 - 2.0 * (rho / (2.0 * EARTH_RADIUS_M)).atan();
            (
                phi * RAD2DEG,
                normalize_lon((self.lambda0 + theta) * RAD2DEG),
            )
        } else {
            let phi = -PI * 0.5 + 2.0 * (rho / (2.0 * EARTH_RADIUS_M)).atan();
            (
                phi * RAD2DEG,
                normalize_lon((self.lambda0 + theta) * RAD2DEG),
            )
        }
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    #[test]
    fn hrrr_lambert_roundtrips_first_grid_point() {
        let projection =
            HrrrLambertProjection::new(38.5, 38.5, 262.5, 21.138123, -122.719528, 3000.0, 3000.0);
        let (lat, lon) = projection.grid_to_latlon(0.0, 0.0);
        assert!((lat - 21.138123).abs() < 0.02, "lat={lat}");
        assert!((lon - (-122.719528)).abs() < 0.02, "lon={lon}");

        let (i, j) = projection.latlon_to_grid(lat, lon);
        assert!(i.abs() < 0.02, "i={i}");
        assert!(j.abs() < 0.02, "j={j}");
    }

    #[test]
    fn hrrr_lambert_bounds_cover_conus_not_pacific_asia() {
        let projection =
            HrrrLambertProjection::new(38.5, 38.5, 262.5, 21.138123, -122.719528, 3000.0, 3000.0);
        let corners = [
            projection.grid_to_latlon(0.0, 0.0),
            projection.grid_to_latlon(1798.0, 0.0),
            projection.grid_to_latlon(0.0, 1058.0),
            projection.grid_to_latlon(1798.0, 1058.0),
        ];
        let lon_min = corners
            .iter()
            .map(|(_, lon)| *lon)
            .fold(f64::INFINITY, f64::min);
        let lon_max = corners
            .iter()
            .map(|(_, lon)| *lon)
            .fold(f64::NEG_INFINITY, f64::max);
        let lat_min = corners
            .iter()
            .map(|(lat, _)| *lat)
            .fold(f64::INFINITY, f64::min);
        let lat_max = corners
            .iter()
            .map(|(lat, _)| *lat)
            .fold(f64::NEG_INFINITY, f64::max);

        assert!((-136.0..=-120.0).contains(&lon_min), "lon_min={lon_min}");
        assert!((-70.0..=-55.0).contains(&lon_max), "lon_max={lon_max}");
        assert!((20.0..=25.0).contains(&lat_min), "lat_min={lat_min}");
        assert!((47.0..=55.0).contains(&lat_max), "lat_max={lat_max}");
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
