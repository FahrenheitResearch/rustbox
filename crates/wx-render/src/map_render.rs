use crate::style::{RenderStyle, format_tick, resolve_render_style};
use crate::text::{draw_text, draw_text_centered, draw_text_right, text_width};
use crate::{MapContourSpec, MapMarker, MapOverlaySpec, MapWindBarbSpec, RenderedOverlay};
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
    render_field_to_map_png_with_layers(field, spec, &spec.contours, &spec.barbs, output_path)
}

pub fn render_field_to_map_png_with_layers(
    field: &Field2D,
    spec: &MapOverlaySpec,
    contours: &[MapContourSpec],
    barbs: &[MapWindBarbSpec],
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
    let style = resolve_render_style(
        &spec.palette,
        spec.value_range,
        spec.levels.as_deref(),
        spec.tick_step,
    )?;
    let layout = PlotLayout {
        x: OUTER_PAD,
        y: OUTER_PAD + TITLE_HEIGHT,
        width: CANVAS_WIDTH - OUTER_PAD * 2 - COLORBAR_WIDTH - COLORBAR_GAP - COLORBAR_LABEL_SPACE,
        height: CANVAS_HEIGHT - OUTER_PAD * 2 - TITLE_HEIGHT - FOOTER_HEIGHT,
    };

    let projected_grid = build_projected_grid(field, layout.width as f64 / layout.height as f64)?;
    let mut image: RgbaImage = ImageBuffer::from_pixel(CANVAS_WIDTH, CANVAS_HEIGHT, PAGE_BG);
    fill_rect(
        &mut image,
        layout.x,
        layout.y,
        layout.width,
        layout.height,
        MAP_BG,
    );

    rasterize_field(&mut image, field, &projected_grid, &style, layout);
    draw_graticule(&mut image, &projected_grid, layout);
    draw_basemap_features(&mut image, &projected_grid, layout);
    draw_contour_overlays(&mut image, field, &projected_grid, contours, layout)?;
    draw_wind_barb_overlays(&mut image, field, &projected_grid, barbs, layout)?;
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

fn validate_compatible_overlay_field(base: &Field2D, overlay: &Field2D, label: &str) -> Result<()> {
    if overlay.grid != base.grid {
        bail!("{label} grid geometry does not match the base field");
    }
    if overlay.values.len() != overlay.expected_len() {
        bail!(
            "{label} value count {} does not match grid {}x{}",
            overlay.values.len(),
            overlay.grid.nx,
            overlay.grid.ny
        );
    }
    Ok(())
}

fn rasterize_field(
    image: &mut RgbaImage,
    field: &Field2D,
    projected_grid: &ProjectedGrid,
    style: &RenderStyle,
    layout: PlotLayout,
) {
    for py in 0..layout.height {
        let grid_y = projected_grid.extent.y_min
            + ((py as f64 + 0.5) / layout.height as f64)
                * (projected_grid.extent.y_max - projected_grid.extent.y_min);
        for px in 0..layout.width {
            let grid_x = projected_grid.extent.x_min
                + ((px as f64 + 0.5) / layout.width as f64)
                    * (projected_grid.extent.x_max - projected_grid.extent.x_min);
            let i = grid_x.round() as isize;
            let j = grid_y.round() as isize;
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
        (layout.y - TITLE_HEIGHT + 2) as i32,
        TITLE_TEXT,
        3,
    );
    draw_text(
        image,
        &subtitle,
        layout.x as i32,
        (layout.y - TITLE_HEIGHT + 30) as i32,
        SUBTITLE_TEXT,
        2,
    );
    if let Some(subtitle_right) = &spec.subtitle_right {
        draw_text_right(
            image,
            subtitle_right,
            (layout.x + layout.width) as i32,
            (layout.y - TITLE_HEIGHT + 30) as i32,
            SUBTITLE_TEXT,
            2,
        );
    }
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
        (layout.y + layout.height + 10) as i32,
        SUBTITLE_TEXT,
        2,
    );
}

fn draw_contour_overlays(
    image: &mut RgbaImage,
    base_field: &Field2D,
    projected_grid: &ProjectedGrid,
    contours: &[MapContourSpec],
    layout: PlotLayout,
) -> Result<()> {
    for contour in contours {
        validate_compatible_overlay_field(base_field, &contour.field, "contour overlay")?;
        draw_single_contour_overlay(image, contour, projected_grid, layout);
    }
    Ok(())
}

fn draw_single_contour_overlay(
    image: &mut RgbaImage,
    contour: &MapContourSpec,
    projected_grid: &ProjectedGrid,
    layout: PlotLayout,
) {
    if contour.levels.is_empty() || contour.field.grid.nx < 2 || contour.field.grid.ny < 2 {
        return;
    }

    let nx = contour.field.grid.nx;
    let ny = contour.field.grid.ny;
    let idx = |j: usize, i: usize| j * nx + i;

    for &level in &contour.levels {
        let mut label_drawn = !contour.labels;
        for j in 0..(ny - 1) {
            for i in 0..(nx - 1) {
                let p0 = (i as f64, j as f64, contour.field.values[idx(j, i)] as f64);
                let p1 = (
                    (i + 1) as f64,
                    j as f64,
                    contour.field.values[idx(j, i + 1)] as f64,
                );
                let p2 = (
                    (i + 1) as f64,
                    (j + 1) as f64,
                    contour.field.values[idx(j + 1, i + 1)] as f64,
                );
                let p3 = (
                    i as f64,
                    (j + 1) as f64,
                    contour.field.values[idx(j + 1, i)] as f64,
                );

                let mut points = Vec::with_capacity(4);
                if let Some(point) = interp_contour_point(p0, p1, level) {
                    points.push(point);
                }
                if let Some(point) = interp_contour_point(p1, p2, level) {
                    points.push(point);
                }
                if let Some(point) = interp_contour_point(p2, p3, level) {
                    points.push(point);
                }
                if let Some(point) = interp_contour_point(p3, p0, level) {
                    points.push(point);
                }

                if points.len() < 2 {
                    continue;
                }

                let segments: &[(usize, usize)] = if points.len() == 4 {
                    &[(0, 1), (2, 3)]
                } else {
                    &[(0, 1)]
                };

                for &(a, b) in segments {
                    let Some(pa) = grid_point_to_canvas(projected_grid, points[a], layout) else {
                        continue;
                    };
                    let Some(pb) = grid_point_to_canvas(projected_grid, points[b], layout) else {
                        continue;
                    };
                    draw_line(image, pa.0, pa.1, pb.0, pb.1, contour.color, contour.width);

                    if contour.labels
                        && !label_drawn
                        && (pb.0 - pa.0).abs() + (pb.1 - pa.1).abs() > 18.0
                    {
                        let label = format_tick(level * contour.label_scale);
                        let tx = ((pa.0 + pb.0) * 0.5) as i32 - text_width(&label, 1) as i32 / 2;
                        let ty = ((pa.1 + pb.1) * 0.5) as i32 - 4;
                        draw_text(image, &label, tx, ty, contour.color, 1);
                        label_drawn = true;
                    }
                }
            }
        }
        if contour.show_extrema {
            draw_contour_extrema(image, contour, projected_grid, layout);
        }
    }
}

fn interp_contour_point(a: (f64, f64, f64), b: (f64, f64, f64), level: f64) -> Option<(f64, f64)> {
    let (x0, y0, v0) = a;
    let (x1, y1, v1) = b;
    if !v0.is_finite() || !v1.is_finite() {
        return None;
    }
    let d0 = v0 - level;
    let d1 = v1 - level;
    if (d0 > 0.0 && d1 > 0.0) || (d0 < 0.0 && d1 < 0.0) {
        return None;
    }
    if (v1 - v0).abs() < 1e-12 {
        return Some(((x0 + x1) * 0.5, (y0 + y1) * 0.5));
    }
    let t = (level - v0) / (v1 - v0);
    Some((x0 + (x1 - x0) * t, y0 + (y1 - y0) * t))
}

fn draw_contour_extrema(
    image: &mut RgbaImage,
    contour: &MapContourSpec,
    projected_grid: &ProjectedGrid,
    layout: PlotLayout,
) {
    let nx = contour.field.grid.nx;
    let ny = contour.field.grid.ny;
    if nx < 21 || ny < 21 {
        return;
    }
    let values = &contour.field.values;
    let idx = |j: usize, i: usize| j * nx + i;
    let mut extrema = Vec::new();
    let window = (ny / 10).clamp(8, 30);
    let edge = (ny / 15).max(6);

    for j in edge..(ny - edge) {
        for i in edge..(nx - edge) {
            let value = values[idx(j, i)];
            if !value.is_finite() {
                continue;
            }
            let mut is_max = true;
            let mut is_min = true;
            let j0 = j.saturating_sub(window);
            let j1 = (j + window).min(ny - 1);
            let i0 = i.saturating_sub(window);
            let i1 = (i + window).min(nx - 1);
            'scan: for jj in j0..=j1 {
                for ii in i0..=i1 {
                    if jj == j && ii == i {
                        continue;
                    }
                    let other = values[idx(jj, ii)];
                    if !other.is_finite() {
                        continue;
                    }
                    if other > value {
                        is_max = false;
                    }
                    if other < value {
                        is_min = false;
                    }
                    if !is_max && !is_min {
                        break 'scan;
                    }
                }
            }
            if is_max {
                extrema.push((i, j, value as f64, "H"));
            } else if is_min {
                extrema.push((i, j, value as f64, "L"));
            }
        }
    }

    let mut kept: Vec<(usize, usize, f64, &str)> = Vec::new();
    for candidate in extrema {
        if kept.iter().all(|(x0, y0, _, _)| {
            let dx = *x0 as f64 - candidate.0 as f64;
            let dy = *y0 as f64 - candidate.1 as f64;
            (dx * dx + dy * dy).sqrt() >= 24.0
        }) {
            kept.push(candidate);
        }
    }

    for (x, y, value, label) in kept.into_iter().take(8) {
        let Some((px, py)) = grid_point_to_canvas(projected_grid, (x as f64, y as f64), layout)
        else {
            continue;
        };
        draw_text_centered(
            image,
            label,
            px.round() as i32,
            py.round() as i32 - 10,
            contour.color,
            2,
        );
        let value_label = format!("{:.0}", value);
        draw_text_centered(
            image,
            &value_label,
            px.round() as i32,
            py.round() as i32 + 8,
            contour.color,
            1,
        );
    }
}

fn draw_wind_barb_overlays(
    image: &mut RgbaImage,
    base_field: &Field2D,
    projected_grid: &ProjectedGrid,
    barbs: &[MapWindBarbSpec],
    layout: PlotLayout,
) -> Result<()> {
    for barb in barbs {
        validate_compatible_overlay_field(base_field, &barb.u_field, "barb u-component")?;
        validate_compatible_overlay_field(base_field, &barb.v_field, "barb v-component")?;
        draw_single_wind_barb_overlay(image, barb, projected_grid, layout);
    }
    Ok(())
}

fn draw_single_wind_barb_overlay(
    image: &mut RgbaImage,
    barb: &MapWindBarbSpec,
    projected_grid: &ProjectedGrid,
    layout: PlotLayout,
) {
    let nx = barb.u_field.grid.nx;
    let ny = barb.u_field.grid.ny;
    let stride_x = barb.stride_x.max(1);
    let stride_y = barb.stride_y.max(1);

    for j in (0..ny).step_by(stride_y) {
        for i in (0..nx).step_by(stride_x) {
            let index = j * nx + i;
            let u = barb.u_field.values[index] as f64 * barb.speed_scale;
            let v = barb.v_field.values[index] as f64 * barb.speed_scale;
            let Some((px, py)) = grid_point_to_canvas(projected_grid, (i as f64, j as f64), layout)
            else {
                continue;
            };
            draw_wind_barb(image, px, py, u, v, barb.color, barb.length_px, barb.width);
        }
    }
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
        layout.y.saturating_sub(26) as i32,
        TITLE_TEXT,
        2,
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
            ty.round() as i32 - 8,
            SUBTITLE_TEXT,
            2,
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
        let Some((px, py)) = projected_grid.extent.pixel_coords(
            marker.grid_x as f64,
            marker.grid_y as f64,
            layout.width,
            layout.height,
        ) else {
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
            draw_text(image, label, label_x, label_y, MARKER_COLOR, 2);
        }
    }
}

fn draw_graticule(image: &mut RgbaImage, projected_grid: &ProjectedGrid, layout: PlotLayout) {
    let (lat_min, lat_max) = projected_grid.lat_bounds();
    let (lon_min, lon_max) = projected_grid.lon_bounds();
    let start_lat = ((lat_min - 2.0) / 5.0).floor() as i32 * 5;
    let end_lat = ((lat_max + 2.0) / 5.0).ceil() as i32 * 5;
    let start_lon = ((lon_min - 2.0) / 5.0).floor() as i32 * 5;
    let end_lon = ((lon_max + 2.0) / 5.0).ceil() as i32 * 5;

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
            let mut projected = Vec::new();
            for (lon, lat) in line {
                if let Some(point) = project_to_pixel(projected_grid, *lat, *lon, layout) {
                    projected.push(point);
                }
            }
            draw_polyline(image, &projected, layer.color, layer.width);
        }
    }
}

fn project_to_pixel(
    projected_grid: &ProjectedGrid,
    lat: f64,
    lon: f64,
    layout: PlotLayout,
) -> Option<(f64, f64)> {
    let (grid_x, native_grid_y) = projected_grid.projector.latlon_to_grid(lat, lon)?;
    let grid_y = projected_grid.native_y_to_display_y(native_grid_y);
    grid_point_to_canvas(projected_grid, (grid_x, grid_y), layout)
}

fn grid_point_to_canvas(
    projected_grid: &ProjectedGrid,
    point: (f64, f64),
    layout: PlotLayout,
) -> Option<(f64, f64)> {
    let (px, py) =
        projected_grid
            .extent
            .pixel_coords(point.0, point.1, layout.width, layout.height)?;
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
    let extent = MapExtent::from_bounds(
        0.0,
        field.grid.nx.saturating_sub(1) as f64,
        0.0,
        field.grid.ny.saturating_sub(1) as f64,
        target_ratio,
    );

    Ok(ProjectedGrid {
        projector,
        extent,
        nx: field.grid.nx,
        ny: field.grid.ny,
        rows_normalized_north_to_south: field.grid.scan_mode & 0x40 != 0,
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

#[derive(Clone, Copy)]
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

    fn pixel_coords(&self, x: f64, y: f64, width: u32, height: u32) -> Option<(f64, f64)> {
        let rx = (x - self.x_min) / (self.x_max - self.x_min);
        let ry = (y - self.y_min) / (self.y_max - self.y_min);
        if !(-0.1..=1.1).contains(&rx) || !(-0.1..=1.1).contains(&ry) {
            return None;
        }
        Some((
            rx * (width.saturating_sub(1)) as f64,
            ry * (height.saturating_sub(1)) as f64,
        ))
    }
}

#[derive(Clone)]
struct ProjectedGrid {
    projector: Projector,
    extent: MapExtent,
    nx: usize,
    ny: usize,
    rows_normalized_north_to_south: bool,
}

impl ProjectedGrid {
    fn lat_bounds(&self) -> (f64, f64) {
        self.sample_geo_bounds()
            .map(|(lat_min, lat_max, _, _)| (lat_min, lat_max))
            .unwrap_or((20.0, 55.0))
    }

    fn lon_bounds(&self) -> (f64, f64) {
        self.sample_geo_bounds()
            .map(|(_, _, lon_min, lon_max)| (lon_min, lon_max))
            .unwrap_or((-130.0, -60.0))
    }

    fn sample_geo_bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let sample_points = [
            (0.0, 0.0),
            (self.nx.saturating_sub(1) as f64, 0.0),
            (0.0, self.ny.saturating_sub(1) as f64),
            (
                self.nx.saturating_sub(1) as f64,
                self.ny.saturating_sub(1) as f64,
            ),
            (self.nx.saturating_sub(1) as f64 / 2.0, 0.0),
            (
                self.nx.saturating_sub(1) as f64 / 2.0,
                self.ny.saturating_sub(1) as f64,
            ),
            (0.0, self.ny.saturating_sub(1) as f64 / 2.0),
            (
                self.nx.saturating_sub(1) as f64,
                self.ny.saturating_sub(1) as f64 / 2.0,
            ),
            (
                self.nx.saturating_sub(1) as f64 / 2.0,
                self.ny.saturating_sub(1) as f64 / 2.0,
            ),
        ];
        let mut lat_min = f64::INFINITY;
        let mut lat_max = f64::NEG_INFINITY;
        let mut lon_min = f64::INFINITY;
        let mut lon_max = f64::NEG_INFINITY;
        let mut any = false;

        for (grid_x, grid_y) in sample_points {
            let native_grid_y = self.display_y_to_native_y(grid_y);
            let Some((lat, lon)) = self.projector.grid_to_latlon(grid_x, native_grid_y) else {
                continue;
            };
            any = true;
            lat_min = lat_min.min(lat);
            lat_max = lat_max.max(lat);
            lon_min = lon_min.min(lon);
            lon_max = lon_max.max(lon);
        }

        any.then_some((lat_min, lat_max, lon_min, lon_max))
    }

    fn native_y_to_display_y(&self, native_y: f64) -> f64 {
        if self.rows_normalized_north_to_south {
            self.ny.saturating_sub(1) as f64 - native_y
        } else {
            native_y
        }
    }

    fn display_y_to_native_y(&self, display_y: f64) -> f64 {
        if self.rows_normalized_north_to_south {
            self.ny.saturating_sub(1) as f64 - display_y
        } else {
            display_y
        }
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

    #[test]
    fn pixel_coords_map_south_to_bottom_and_north_to_top() {
        let extent = MapExtent::from_bounds(0.0, 10.0, 0.0, 10.0, 1.0);

        let (_, south_y) = extent
            .pixel_coords(0.0, 0.0, 100, 100)
            .expect("south point");
        let (_, north_y) = extent
            .pixel_coords(0.0, 10.0, 100, 100)
            .expect("north point");

        assert!(south_y < north_y, "south_y={south_y} north_y={north_y}");
        assert!(south_y.abs() < 0.1, "south_y={south_y}");
        assert!((north_y - 99.0).abs() < 0.1, "north_y={north_y}");
    }

    #[test]
    fn normalized_hrrr_rows_flip_native_y_for_projection() {
        let projection =
            HrrrLambertProjection::new(38.5, 38.5, 262.5, 21.138123, -122.719528, 3000.0, 3000.0);
        let projected = ProjectedGrid {
            projector: Projector::Lambert(projection),
            extent: MapExtent::from_bounds(0.0, 1798.0, 0.0, 1058.0, 1.0),
            nx: 1799,
            ny: 1059,
            rows_normalized_north_to_south: true,
        };

        let (north_lat, _) = projected
            .projector
            .grid_to_latlon(0.0, projected.display_y_to_native_y(0.0))
            .expect("north row");
        let (south_lat, _) = projected
            .projector
            .grid_to_latlon(0.0, projected.display_y_to_native_y(1058.0))
            .expect("south row");

        assert!(
            north_lat > south_lat,
            "north_lat={north_lat} south_lat={south_lat}"
        );
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

#[allow(clippy::too_many_arguments)]
fn draw_wind_barb(
    image: &mut RgbaImage,
    x_tip: f64,
    y_tip: f64,
    u: f64,
    v: f64,
    color: Rgba<u8>,
    shaft_len: f64,
    width: u32,
) {
    if !u.is_finite() || !v.is_finite() {
        return;
    }

    let speed = (u * u + v * v).sqrt();
    if speed < 2.5 {
        blend_pixel(image, x_tip.round() as i32, y_tip.round() as i32, color);
        return;
    }

    let tail_dx = -u / speed;
    let tail_dy = v / speed;
    let perp_dx = -tail_dy;
    let perp_dy = tail_dx;

    let tail_x = x_tip + tail_dx * shaft_len;
    let tail_y = y_tip + tail_dy * shaft_len;
    draw_line(image, tail_x, tail_y, x_tip, y_tip, color, width);

    let mut remaining = ((speed + 2.5) / 5.0).floor() as i32 * 5;
    let mut offset = shaft_len;
    let spacing = (shaft_len * 0.16).max(2.0);
    let full_height = shaft_len * 0.40;
    let full_width = shaft_len * 0.25;

    while remaining >= 50 {
        draw_barb_flag(
            image,
            (x_tip, y_tip),
            (tail_dx, tail_dy),
            (perp_dx, perp_dy),
            offset,
            full_height,
            full_width,
            color,
            width,
        );
        remaining -= 50;
        offset -= full_width + spacing;
    }

    while remaining >= 10 {
        draw_barb_segment(
            image,
            (x_tip, y_tip),
            (tail_dx, tail_dy),
            (perp_dx, perp_dy),
            offset,
            full_height,
            full_width * 0.5,
            color,
            width,
        );
        remaining -= 10;
        offset -= spacing;
    }

    if remaining >= 5 {
        if (offset - shaft_len).abs() < 1e-6 {
            offset -= 1.5 * spacing;
        }
        draw_barb_segment(
            image,
            (x_tip, y_tip),
            (tail_dx, tail_dy),
            (perp_dx, perp_dy),
            offset,
            full_height * 0.5,
            full_width * 0.25,
            color,
            width,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_barb_segment(
    image: &mut RgbaImage,
    tip: (f64, f64),
    tail_dir: (f64, f64),
    perp_dir: (f64, f64),
    offset: f64,
    height: f64,
    along_tail: f64,
    color: Rgba<u8>,
    width: u32,
) {
    let base_x = tip.0 + tail_dir.0 * offset;
    let base_y = tip.1 + tail_dir.1 * offset;
    let feather_x = base_x + perp_dir.0 * height + tail_dir.0 * along_tail;
    let feather_y = base_y + perp_dir.1 * height + tail_dir.1 * along_tail;
    draw_line(image, base_x, base_y, feather_x, feather_y, color, width);
}

#[allow(clippy::too_many_arguments)]
fn draw_barb_flag(
    image: &mut RgbaImage,
    tip: (f64, f64),
    tail_dir: (f64, f64),
    perp_dir: (f64, f64),
    offset: f64,
    height: f64,
    along_tail: f64,
    color: Rgba<u8>,
    width: u32,
) {
    let base_x = tip.0 + tail_dir.0 * offset;
    let base_y = tip.1 + tail_dir.1 * offset;
    let flag_tip_x = base_x + perp_dir.0 * height - tail_dir.0 * (along_tail * 0.5);
    let flag_tip_y = base_y + perp_dir.1 * height - tail_dir.1 * (along_tail * 0.5);
    let flag_tail_x = base_x - tail_dir.0 * along_tail;
    let flag_tail_y = base_y - tail_dir.1 * along_tail;
    draw_line(
        image,
        base_x,
        base_y,
        flag_tip_x,
        flag_tip_y,
        color,
        width + 1,
    );
    draw_line(
        image,
        flag_tip_x,
        flag_tip_y,
        flag_tail_x,
        flag_tail_y,
        color,
        width + 1,
    );
    draw_line(
        image,
        flag_tail_x,
        flag_tail_y,
        base_x,
        base_y,
        color,
        width + 1,
    );
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
