use anyhow::Result;
use wx_types::LevelDescriptor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    pub model: String,
    pub cycle: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPlan {
    pub source: String,
    pub subset_index: String,
    pub requested_fields: Vec<String>,
    pub levels: Vec<LevelDescriptorLite>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelDescriptorLite {
    pub name: String,
    pub units: String,
}

impl From<&LevelDescriptor> for LevelDescriptorLite {
    fn from(value: &LevelDescriptor) -> Self {
        Self {
            name: value.name.clone(),
            units: value.units.clone(),
        }
    }
}

pub fn build_hrrr_plan(request: &FetchRequest, levels: &[LevelDescriptor]) -> Result<FetchPlan> {
    Ok(FetchPlan {
        source: "nomads".to_string(),
        subset_index: format!("{}-{}", request.model, request.cycle),
        requested_fields: request.fields.clone(),
        levels: levels.iter().map(LevelDescriptorLite::from).collect(),
    })
}

