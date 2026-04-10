use anyhow::{Context, Result, bail};
use image::RgbaImage;
use std::path::{Path, PathBuf};
use wx_radar::{
    ColorTablePreset, RadarProduct, RenderMode, available_products, detect_signatures,
    read_level2_file, render_product, summarize_volume,
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("status");

    match command {
        "status" => print_status(),
        "inspect" => run_inspect(&args[1..])?,
        "render" => run_render(&args[1..])?,
        "detect" => run_detect(&args[1..])?,
        _ => print_usage(),
    }

    Ok(())
}

fn print_status() {
    println!(
        "wx-radar: real rustdar-backed Level II parser, derived products, detection, and render core"
    );
    println!(
        "radar-viewer: thin real CLI over wx-radar for inspect/render/detect on local Level II files"
    );
}

fn run_inspect(args: &[String]) -> Result<()> {
    let input = required_path(args.first(), "inspect requires <level2_file>")?;
    let file = read_level2_file(&input)?;
    let summary = summarize_volume(&file);
    let products: Vec<String> = available_products(&file)
        .into_iter()
        .map(|product| product.short_name().to_string())
        .collect();
    let payload = serde_json::json!({
        "summary": summary,
        "products": products,
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

fn run_render(args: &[String]) -> Result<()> {
    if args.len() < 3 {
        bail!(
            "render requires <level2_file> <product> <output_png> [sweep_index] [size] [mode] [preset]"
        );
    }
    let input = PathBuf::from(&args[0]);
    let product = parse_product(&args[1])?;
    let output = PathBuf::from(&args[2]);
    let sweep_index = parse_optional_usize(args.get(3), 0, "sweep_index")?;
    let size = parse_optional_u32(args.get(4), 1024, "size")?;
    let mode = parse_mode(args.get(5).map(String::as_str).unwrap_or("classic"))?;
    let preset = parse_preset(args.get(6).map(String::as_str).unwrap_or("default"))?;

    let file = read_level2_file(&input)?;
    let rendered = render_product(&file, product, sweep_index, size, mode, preset)?;
    save_png(&output, rendered.width, rendered.height, rendered.pixels)?;

    println!(
        "rendered site={} product={} sweep_index={} size={} mode={:?} preset={:?} output={}",
        file.station_id,
        product.short_name(),
        sweep_index,
        size,
        mode,
        preset,
        output.display()
    );
    Ok(())
}

fn run_detect(args: &[String]) -> Result<()> {
    let input = required_path(args.first(), "detect requires <level2_file>")?;
    let file = read_level2_file(&input)?;
    let summary = detect_signatures(&file)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn required_path(value: Option<&String>, message: &str) -> Result<PathBuf> {
    value
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!(message.to_string()))
}

fn parse_product(value: &str) -> Result<RadarProduct> {
    let uppercase = value.to_uppercase();
    let product = match uppercase.as_str() {
        "ET" | "ECHOTOPS" | "ECHO_TOPS" => RadarProduct::EchoTops,
        other => RadarProduct::from_name(other),
    };
    if product == RadarProduct::Unknown {
        bail!("unknown radar product '{}'", value);
    }
    Ok(product)
}

fn parse_mode(value: &str) -> Result<RenderMode> {
    match value.to_ascii_lowercase().as_str() {
        "classic" => Ok(RenderMode::Classic),
        "smooth" => Ok(RenderMode::Smooth),
        _ => bail!("unknown render mode '{}'", value),
    }
}

fn parse_preset(value: &str) -> Result<ColorTablePreset> {
    match value.to_ascii_lowercase().as_str() {
        "default" => Ok(ColorTablePreset::Default),
        "gr2analyst" | "gr2a" => Ok(ColorTablePreset::GR2Analyst),
        "nssl" => Ok(ColorTablePreset::NSSL),
        "classic" => Ok(ColorTablePreset::Classic),
        "dark" => Ok(ColorTablePreset::Dark),
        "colorblind" => Ok(ColorTablePreset::Colorblind),
        _ => bail!("unknown color-table preset '{}'", value),
    }
}

fn parse_optional_usize(value: Option<&String>, default: usize, label: &str) -> Result<usize> {
    match value {
        Some(value) => value
            .parse::<usize>()
            .with_context(|| format!("failed to parse {} '{}'", label, value)),
        None => Ok(default),
    }
}

fn parse_optional_u32(value: Option<&String>, default: u32, label: &str) -> Result<u32> {
    match value {
        Some(value) => value
            .parse::<u32>()
            .with_context(|| format!("failed to parse {} '{}'", label, value)),
        None => Ok(default),
    }
}

fn save_png(path: &Path, width: u32, height: u32, pixels: Vec<u8>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let image = RgbaImage::from_raw(width, height, pixels)
        .context("rendered RGBA buffer shape did not match image dimensions")?;
    image
        .save(path)
        .with_context(|| format!("failed to save PNG {}", path.display()))?;
    Ok(())
}

fn print_usage() {
    println!("usage:");
    println!("  cargo run -p radar-viewer-app -- status");
    println!("  cargo run -p radar-viewer-app -- inspect <level2_file>");
    println!(
        "  cargo run -p radar-viewer-app -- render <level2_file> <product> <output_png> [sweep_index] [size] [mode] [preset]"
    );
    println!("  cargo run -p radar-viewer-app -- detect <level2_file>");
    println!("products: REF VEL SW ZDR CC PHI KDP VIL ET SRV");
}
