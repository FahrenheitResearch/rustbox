use wx_render::OverlaySpec;
use wx_severe::SevereDiagnostics;

fn main() {
    let overlay = OverlaySpec {
        palette: "winds".to_string(),
        transparent_background: true,
        value_range: None,
    };
    let severe = SevereDiagnostics::default();
    println!(
        "mesoanalysis app scaffold: palette={} stp={:.1}",
        overlay.palette, severe.significant_tornado_parameter
    );
}
