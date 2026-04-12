use image::Rgba;

const WINDS: &[&str] = &[
    "#ffffff", "#87cefa", "#6a5acd", "#e696dc", "#c85abe", "#a01496", "#c80028", "#dc283c",
    "#f05050", "#faf064", "#dcbe46", "#be8c28", "#a05a0a",
];

const TEMPERATURE: &[&str] = &[
    "#2b5d7e", "#75a8b0", "#aee3dc", "#a0b8d6", "#968bc5", "#8243b2", "#a343b3", "#f7f7ff",
    "#a0b8d6", "#0f5575", "#6d8c77", "#f8eea2", "#aa714d", "#5f0000", "#852c40", "#b28f85",
    "#e7e0da", "#959391", "#454844",
];

const RELVORT: &[&str] = &[
    "#323232", "#4d4d4d", "#707070", "#8A8A8A", "#a1a1a1", "#c0c0c0", "#d6d6d6", "#e5e5e5",
    "#ffffff", "#fdd244", "#fea000", "#f16702", "#da2422", "#ab029b", "#78008f", "#44008b",
    "#000160", "#244488", "#4f85b2", "#73cadb", "#91fffd",
];

const REFLECTIVITY: &[&str] = &[
    "#ffffff", "#f2f6fc", "#d9e3f4", "#b0c6e6", "#8aa7da", "#648bcb", "#396dc1", "#1350b4",
    "#0d4f5d", "#43736f", "#77987b", "#a8bf8b", "#fdf273", "#f2d45a", "#eeb247", "#e1932d",
    "#d97517", "#cd5403", "#cd0002", "#a10206", "#75030b", "#9e37ab", "#83259d", "#601490",
    "#818181", "#b3b3b3", "#e8e8e8",
];

const COMP_SEG0: &[&str] = &["#ffffff", "#696969"];
const COMP_SEG1: &[&str] = &["#37536a", "#a7c8ce"];
const COMP_SEG2: &[&str] = &["#e9dd96", "#e16f02"];
const COMP_SEG3: &[&str] = &["#dc4110", "#8b0950"];
const COMP_SEG4: &[&str] = &["#73088a", "#da99e7"];
const COMP_SEG5: &[&str] = &["#e9bec3", "#b2445a"];
const COMP_SEG6: &[&str] = &["#893d48", "#bc9195"];

const DIVERGENCE: &[&str] = &[
    "#113d6b", "#2d6ea0", "#6aa4c8", "#d7ebf5", "#f7f7f7", "#f3d4c7", "#d98b6f", "#b54837",
    "#7d1822",
];
const ADVECTION: &[&str] = &[
    "#0b3c5d", "#328cc1", "#74b3ce", "#d9ecf2", "#f7f7f7", "#f3d9ca", "#e39b7b", "#c75d43",
    "#8f2d1f",
];
const FRONTOGENESIS: &[&str] = &[
    "#183a63", "#2e5f92", "#5f90bf", "#a9c9e6", "#f7f7f7", "#f0c7b8", "#da8f78", "#c45c48",
    "#9e2f2f", "#6f0f1f",
];

const DEWPOINT_DRY: &[&str] = &["#996f4f", "#4d4236", "#f2f2d8"];
const DEWPOINT_MOIST: &[(&[&str], usize)] = &[
    (&["#e3f3e6", "#64c461"], 10),
    (&["#32ae32", "#084d06"], 10),
    (&["#66a3ad", "#12292a"], 10),
    (&["#66679d", "#2b1e63"], 10),
    (&["#714270", "#a27382"], 10),
];

const RH_SEG1: &[&str] = &["#a5734d", "#382f28", "#6e6559", "#a59b8e", "#ddd1c3"];
const RH_SEG2: &[&str] = &["#c8d7c0", "#004a2f"];
const RH_SEG3: &[&str] = &["#004123", "#28588c"];

const SIM_IR_COOL: &[&str] = &["#7f017f", "#e36fbe"];
const SIM_IR_WARM: &[&str] = &[
    "#FFFFFF", "#000000", "#fd0100", "#fcff05", "#03fd03", "#010077", "#0ff6ef",
];
const SIM_IR_GRAY: &[&str] = &["#ffffff", "#000000"];

const GEOPOT_ANOMALY: &[&str] = &[
    "#c9f2fc", "#e684f4", "#732164", "#7b2b8d", "#8a41d6", "#253fba", "#7089cb", "#c0d5e8",
    "#ffffff", "#fbcfa1", "#fc984b", "#b83800", "#a3241a", "#5e1425", "#42293e", "#557b75",
    "#ddd5cf",
];

const PRECIP_SEGS: &[(&[&str], usize)] = &[
    (&["#ffffff", "#ffffff"], 1),
    (&["#dcdcdc", "#bebebe", "#9e9e9e", "#818181"], 9),
    (&["#b8f0c1", "#156471"], 40),
    (&["#164fba", "#d8edf5"], 50),
    (&["#cfbddd", "#a134b1"], 100),
    (&["#a43c32", "#dd9c98"], 200),
    (&["#f6f0a3", "#7e4b26", "#542f17"], 1100),
];

pub fn reflectivity() -> Vec<Rgba<u8>> {
    REFLECTIVITY
        .iter()
        .map(|value| rgba_from_hex(value))
        .collect()
}

pub fn winds(count: usize) -> Vec<Rgba<u8>> {
    lerp_hex(WINDS, count)
}

pub fn temperature(count: usize) -> Vec<Rgba<u8>> {
    lerp_hex(TEMPERATURE, count)
}

pub fn dewpoint(dry: usize, moist_points_total: usize) -> Vec<Rgba<u8>> {
    let mut colors = lerp_hex(DEWPOINT_DRY, dry);
    let moist_per_segment = moist_points_total / DEWPOINT_MOIST.len().max(1);
    for (anchors, _) in DEWPOINT_MOIST {
        colors.extend(lerp_hex(anchors, moist_per_segment));
    }
    colors
}

pub fn rh() -> Vec<Rgba<u8>> {
    let mut colors = lerp_hex(RH_SEG1, 40);
    colors.extend(lerp_hex(RH_SEG2, 50));
    colors.extend(lerp_hex(RH_SEG3, 10));
    colors
}

pub fn relative_vorticity(count: usize) -> Vec<Rgba<u8>> {
    lerp_hex(RELVORT, count)
}

pub fn sim_ir() -> Vec<Rgba<u8>> {
    let mut colors = lerp_hex(SIM_IR_COOL, 10);
    colors.extend(lerp_hex(SIM_IR_WARM, 60));
    colors.extend(lerp_hex(SIM_IR_GRAY, 60));
    colors
}

pub fn divergence(count: usize) -> Vec<Rgba<u8>> {
    lerp_hex(DIVERGENCE, count)
}

pub fn advection(count: usize) -> Vec<Rgba<u8>> {
    lerp_hex(ADVECTION, count)
}

pub fn frontogenesis(count: usize) -> Vec<Rgba<u8>> {
    lerp_hex(FRONTOGENESIS, count)
}

pub fn three_cape() -> Vec<Rgba<u8>> {
    build_composite(&[10, 10, 10, 10, 10, 10, 40])
}

pub fn ehi() -> Vec<Rgba<u8>> {
    build_composite(&[10, 10, 20, 20, 20, 40, 40])
}

pub fn lapse_rate() -> Vec<Rgba<u8>> {
    build_composite(&[40, 10, 10, 10, 10, 0, 0])
}

pub fn uh() -> Vec<Rgba<u8>> {
    build_composite(&[10, 10, 10, 10, 20, 20, 0])
}

pub fn geopot_anomaly(count: usize) -> Vec<Rgba<u8>> {
    lerp_hex(GEOPOT_ANOMALY, count)
}

pub fn precip_in() -> Vec<Rgba<u8>> {
    build_segments(PRECIP_SEGS)
}

pub fn shaded_overlay() -> Vec<Rgba<u8>> {
    vec![Rgba([0, 0, 0, 0]), Rgba([0, 0, 0, 0x60])]
}

pub fn cape() -> Vec<Rgba<u8>> {
    build_composite(&[10, 10, 10, 10, 10, 10, 20])
}

pub fn srh() -> Vec<Rgba<u8>> {
    build_composite(&[10, 10, 10, 10, 10, 10, 40])
}

pub fn stp() -> Vec<Rgba<u8>> {
    build_composite(&[10, 10, 10, 10, 10, 10, 40])
}

fn build_composite(quants: &[usize; 7]) -> Vec<Rgba<u8>> {
    let segments: [&[&str]; 7] = [
        COMP_SEG0, COMP_SEG1, COMP_SEG2, COMP_SEG3, COMP_SEG4, COMP_SEG5, COMP_SEG6,
    ];
    let mut colors = Vec::new();
    for (segment, count) in segments.iter().zip(quants.iter()) {
        if *count > 0 {
            colors.extend(lerp_hex(segment, *count));
        }
    }
    colors
}

fn build_segments(segs: &[(&[&str], usize)]) -> Vec<Rgba<u8>> {
    let mut colors = Vec::new();
    for (anchors, count) in segs {
        if *count > 0 {
            colors.extend(lerp_hex(anchors, *count));
        }
    }
    colors
}

fn lerp_hex(anchors: &[&str], count: usize) -> Vec<Rgba<u8>> {
    if count == 0 || anchors.is_empty() {
        return Vec::new();
    }
    if anchors.len() == 1 {
        return std::iter::repeat_n(rgba_from_hex(anchors[0]), count).collect();
    }

    let anchor_colors: Vec<[u8; 4]> = anchors.iter().map(|value| rgba_from_hex(value).0).collect();
    (0..count)
        .map(|index| {
            let t = if count <= 1 {
                0.0
            } else {
                index as f64 / (count - 1) as f64
            };
            let position = t * (anchor_colors.len().saturating_sub(1)) as f64;
            let lower_index = position.floor() as usize;
            let upper_index = lower_index.min(anchor_colors.len() - 2) + 1;
            let fraction = (position - lower_index as f64).clamp(0.0, 1.0);
            let lower = anchor_colors[lower_index.min(anchor_colors.len() - 1)];
            let upper = anchor_colors[upper_index.min(anchor_colors.len() - 1)];
            Rgba([
                lerp_channel(lower[0], upper[0], fraction),
                lerp_channel(lower[1], upper[1], fraction),
                lerp_channel(lower[2], upper[2], fraction),
                255,
            ])
        })
        .collect()
}

fn lerp_channel(start: u8, end: u8, fraction: f64) -> u8 {
    (start as f64 + (end as f64 - start as f64) * fraction)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn rgba_from_hex(value: &str) -> Rgba<u8> {
    let trimmed = value.trim_start_matches('#');
    let red = u8::from_str_radix(&trimmed[0..2], 16).expect("valid red channel");
    let green = u8::from_str_radix(&trimmed[2..4], 16).expect("valid green channel");
    let blue = u8::from_str_radix(&trimmed[4..6], 16).expect("valid blue channel");
    Rgba([red, green, blue, 255])
}
