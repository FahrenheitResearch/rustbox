use font8x8::UnicodeFonts;
use image::{Rgba, RgbaImage};
use rusttype::{Font, Scale, point};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

struct FontSet {
    regular: Option<Font<'static>>,
    bold: Option<Font<'static>>,
}

#[derive(Clone, Copy)]
enum FontKind {
    Regular,
    Bold,
}

static FONTS: OnceLock<FontSet> = OnceLock::new();

pub fn draw_text(img: &mut RgbaImage, text: &str, x: i32, y: i32, color: Rgba<u8>, scale: u32) {
    draw_text_inner(img, text, x, y, color, scale, FontKind::Regular);
}

pub fn draw_text_centered(
    img: &mut RgbaImage,
    text: &str,
    center_x: i32,
    y: i32,
    color: Rgba<u8>,
    scale: u32,
) {
    let width = text_width_with_kind(text, scale, FontKind::Bold) as i32;
    draw_text_inner(
        img,
        text,
        center_x - width / 2,
        y,
        color,
        scale,
        FontKind::Bold,
    );
}

pub fn draw_text_right(
    img: &mut RgbaImage,
    text: &str,
    x_right: i32,
    y: i32,
    color: Rgba<u8>,
    scale: u32,
) {
    let width = text_width(text, scale) as i32;
    draw_text_inner(
        img,
        text,
        x_right - width,
        y,
        color,
        scale,
        FontKind::Regular,
    );
}

pub fn text_width(text: &str, scale: u32) -> u32 {
    text_width_with_kind(text, scale, FontKind::Regular)
}

fn draw_text_inner(
    img: &mut RgbaImage,
    text: &str,
    x: i32,
    y: i32,
    color: Rgba<u8>,
    scale: u32,
    kind: FontKind,
) {
    if let Some(font) = get_font(kind) {
        draw_ttf_halo(img, text, x, y, color, scale, font, kind);
        draw_ttf_text(img, text, x, y, color, scale, font, kind);
    } else {
        draw_bitmap_text(img, text, x, y, color, scale);
    }
}

fn text_width_with_kind(text: &str, scale: u32, kind: FontKind) -> u32 {
    if let Some(font) = get_font(kind) {
        let font_scale = Scale::uniform(font_size_px(scale, kind));
        let metrics = font.v_metrics(font_scale);
        let glyphs: Vec<_> = font
            .layout(text, font_scale, point(0.0, metrics.ascent))
            .collect();
        glyphs
            .iter()
            .rev()
            .find_map(|glyph| {
                glyph
                    .pixel_bounding_box()
                    .map(|bbox| bbox.max.x.max(0) as u32)
            })
            .or_else(|| {
                glyphs.last().map(|glyph| {
                    let end = glyph.position().x + glyph.unpositioned().h_metrics().advance_width;
                    end.max(0.0).ceil() as u32
                })
            })
            .unwrap_or(0)
    } else {
        text.chars().count() as u32 * 8 * scale.max(1)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_ttf_text(
    img: &mut RgbaImage,
    text: &str,
    x: i32,
    y: i32,
    color: Rgba<u8>,
    scale_tag: u32,
    font: &Font<'static>,
    kind: FontKind,
) {
    let font_scale = Scale::uniform(font_size_px(scale_tag, kind));
    let metrics = font.v_metrics(font_scale);
    let glyphs = font.layout(text, font_scale, point(x as f32, y as f32 + metrics.ascent));

    for glyph in glyphs {
        if let Some(bbox) = glyph.pixel_bounding_box() {
            glyph.draw(|gx, gy, coverage| {
                let px = bbox.min.x + gx as i32;
                let py = bbox.min.y + gy as i32;
                let alpha = (color.0[3] as f32 * coverage).round().clamp(0.0, 255.0) as u8;
                blend_pixel(
                    img,
                    px,
                    py,
                    Rgba([color.0[0], color.0[1], color.0[2], alpha]),
                );
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_ttf_halo(
    img: &mut RgbaImage,
    text: &str,
    x: i32,
    y: i32,
    color: Rgba<u8>,
    scale_tag: u32,
    font: &Font<'static>,
    kind: FontKind,
) {
    let halo = halo_color(color);
    let radius = if scale_tag >= 3 { 2 } else { 1 };
    for oy in -radius..=radius {
        for ox in -radius..=radius {
            if ox == 0 && oy == 0 {
                continue;
            }
            draw_ttf_text(img, text, x + ox, y + oy, halo, scale_tag, font, kind);
        }
    }
}

fn draw_bitmap_text(img: &mut RgbaImage, text: &str, x: i32, y: i32, color: Rgba<u8>, scale: u32) {
    let scale = scale.max(1);
    let glyph_width = 8 * scale;

    for (index, character) in text.chars().enumerate() {
        let glyph = get_bitmap_glyph(character);
        let glyph_x = x + index as i32 * glyph_width as i32;

        for row in 0..8u32 {
            let bits = glyph[row as usize];
            for col in 0..8u32 {
                if bits & (1 << col) == 0 {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px = glyph_x + (col * scale + sx) as i32;
                        let py = y + (row * scale + sy) as i32;
                        blend_pixel(img, px, py, color);
                    }
                }
            }
        }
    }
}

fn get_bitmap_glyph(character: char) -> [u8; 8] {
    if (character as u32) < 128 {
        font8x8::BASIC_FONTS.get(character).unwrap_or([0; 8])
    } else {
        [0; 8]
    }
}

fn blend_pixel(img: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x < 0 || y < 0 || (x as u32) >= img.width() || (y as u32) >= img.height() {
        return;
    }
    if color.0[3] == 0 {
        return;
    }
    if color.0[3] == 255 {
        img.put_pixel(x as u32, y as u32, color);
        return;
    }

    let dst = img.get_pixel(x as u32, y as u32).0;
    let alpha = color.0[3] as f64 / 255.0;
    let inv = 1.0 - alpha;
    img.put_pixel(
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

fn font_size_px(scale: u32, kind: FontKind) -> f32 {
    match (scale.max(1), kind) {
        (1, FontKind::Regular) => 13.0,
        (1, FontKind::Bold) => 16.0,
        (2, FontKind::Regular) => 18.0,
        (2, FontKind::Bold) => 22.0,
        (value, FontKind::Regular) => 13.0 + (value as f32 - 1.0) * 5.0,
        (value, FontKind::Bold) => 16.0 + (value as f32 - 1.0) * 5.5,
    }
}

fn halo_color(color: Rgba<u8>) -> Rgba<u8> {
    let luminance =
        0.2126 * color.0[0] as f32 + 0.7152 * color.0[1] as f32 + 0.0722 * color.0[2] as f32;
    if luminance > 140.0 {
        Rgba([22, 28, 36, 168])
    } else {
        Rgba([255, 255, 255, 184])
    }
}

fn get_font(kind: FontKind) -> Option<&'static Font<'static>> {
    let fonts = FONTS.get_or_init(load_fonts);
    match kind {
        FontKind::Regular => fonts.regular.as_ref(),
        FontKind::Bold => fonts.bold.as_ref().or(fonts.regular.as_ref()),
    }
}

fn load_fonts() -> FontSet {
    FontSet {
        regular: load_font_candidates(false),
        bold: load_font_candidates(true),
    }
}

fn load_font_candidates(bold: bool) -> Option<Font<'static>> {
    for path in font_candidates(bold) {
        if let Ok(bytes) = fs::read(&path)
            && let Some(font) = Font::try_from_vec(bytes)
        {
            return Some(font);
        }
    }
    None
}

fn font_candidates(bold: bool) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let env_key = if bold {
        "RUSTBOX_RENDER_FONT_BOLD"
    } else {
        "RUSTBOX_RENDER_FONT_REGULAR"
    };
    if let Ok(path) = env::var(env_key) {
        candidates.push(PathBuf::from(path));
    }

    candidates.push(PathBuf::from(r"C:\Windows\Fonts").join(if bold {
        "segoeuib.ttf"
    } else {
        "segoeui.ttf"
    }));
    candidates.push(PathBuf::from(r"C:\Windows\Fonts").join(if bold {
        "arialbd.ttf"
    } else {
        "arial.ttf"
    }));

    if let Ok(home) = env::var("USERPROFILE") {
        let home = PathBuf::from(home);
        let matplotlib = home
            .join("AppData")
            .join("Roaming")
            .join("Python")
            .join("Python313")
            .join("site-packages")
            .join("matplotlib")
            .join("mpl-data")
            .join("fonts")
            .join("ttf");
        candidates.push(matplotlib.join(if bold {
            "DejaVuSans-Bold.ttf"
        } else {
            "DejaVuSans.ttf"
        }));
        candidates.push(
            home.join("AppData")
                .join("Local")
                .join("Microsoft")
                .join("Windows")
                .join("Fonts")
                .join(if bold {
                    "DejaVuSans-Bold.ttf"
                } else {
                    "DejaVuSans.ttf"
                }),
        );
    }

    candidates.push(
        PathBuf::from(r"C:\Python313\Lib\site-packages\matplotlib\mpl-data\fonts\ttf").join(
            if bold {
                "DejaVuSans-Bold.ttf"
            } else {
                "DejaVuSans.ttf"
            },
        ),
    );

    candidates
}
