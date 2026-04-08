use wx_severe::SevereDiagnostics;
use wx_types::FieldGrid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlaySpec {
    pub palette: String,
    pub transparent_background: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedOverlay {
    pub label: String,
    pub width: usize,
    pub height: usize,
}

pub fn render_overlay(
    field: &FieldGrid,
    severe: &SevereDiagnostics,
    spec: &OverlaySpec,
) -> RenderedOverlay {
    let label = format!(
        "{} [{}] stp={:.2}",
        field.name, spec.palette, severe.significant_tornado_parameter
    );

    RenderedOverlay {
        label,
        width: field.nx,
        height: field.ny,
    }
}

