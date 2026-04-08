use wx_radar::RadarVolume;

fn main() {
    let volume = RadarVolume {
        site: "KTLX".to_string(),
        sweep_count: 0,
    };
    println!("radar viewer scaffold: {}", volume.summary());
}

