use font8x8::UnicodeFonts;
use image::{Rgba, RgbaImage};

pub fn draw_text(img: &mut RgbaImage, text: &str, x: i32, y: i32, color: Rgba<u8>, scale: u32) {
    let scale = scale.max(1);
    let glyph_w = 8 * scale;

    for (index, ch) in text.chars().enumerate() {
        let glyph = if (ch as u32) < 128 {
            font8x8::BASIC_FONTS.get(ch).unwrap_or([0u8; 8])
        } else {
            [0u8; 8]
        };
        let glyph_x = x + index as i32 * glyph_w as i32;

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
                        if px >= 0
                            && py >= 0
                            && (px as u32) < img.width()
                            && (py as u32) < img.height()
                        {
                            img.put_pixel(px as u32, py as u32, color);
                        }
                    }
                }
            }
        }
    }
}

pub fn draw_text_centered(
    img: &mut RgbaImage,
    text: &str,
    center_x: i32,
    y: i32,
    color: Rgba<u8>,
    scale: u32,
) {
    let width = text_width(text, scale) as i32;
    draw_text(img, text, center_x - width / 2, y, color, scale);
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
    draw_text(img, text, x_right - width, y, color, scale);
}

pub fn text_width(text: &str, scale: u32) -> u32 {
    text.chars().count() as u32 * 8 * scale.max(1)
}
