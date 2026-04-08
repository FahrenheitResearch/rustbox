use wx_render::OverlaySpec;
use wx_severe::SevereDiagnostics;

fn main() {
    let overlay = OverlaySpec {
        palette: "cape".to_string(),
        transparent_background: true,
    };
    let severe = SevereDiagnostics {
        significant_tornado_parameter: 0.0,
    };
    println!(
        "mesoanalysis app scaffold: palette={} stp={:.1}",
        overlay.palette, severe.significant_tornado_parameter
    );
}
