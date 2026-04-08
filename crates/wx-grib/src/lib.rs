use anyhow::Result;
use wx_fetch::FetchPlan;
use wx_types::{FieldGrid, FieldKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GribSubsetRequest {
    pub variable: String,
    pub level: String,
}

pub fn decode_subset(plan: &FetchPlan, request: &GribSubsetRequest) -> Result<FieldGrid> {
    let values = vec![0.0_f32; 16];
    Ok(FieldGrid {
        name: format!("{}:{}", request.variable, request.level),
        units: format!("decoded-from-{}", plan.source),
        kind: FieldKind::Scalar,
        nx: 4,
        ny: 4,
        values,
    })
}

