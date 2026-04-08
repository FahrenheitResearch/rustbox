use wx_grid::LayerKinematics;
use wx_thermo::ParcelDiagnostics;

#[derive(Debug, Clone, PartialEq)]
pub struct SevereDiagnostics {
    pub significant_tornado_parameter: f32,
}

pub fn compose_significant_tornado_parameter(
    parcel: &ParcelDiagnostics,
    layer: &LayerKinematics,
) -> SevereDiagnostics {
    let stp = (parcel.mlcape_jkg / 1_000.0).max(0.0)
        * (layer.srh_01km_m2s2 / 100.0).max(0.0)
        * (layer.bulk_shear_06km_ms / 10.0).max(0.0);

    SevereDiagnostics {
        significant_tornado_parameter: stp,
    }
}
