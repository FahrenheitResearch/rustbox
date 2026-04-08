use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    Scalar,
    VectorU,
    VectorV,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelDescriptor {
    pub name: String,
    pub value: f32,
    pub units: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldGrid {
    pub name: String,
    pub units: String,
    pub kind: FieldKind,
    pub nx: usize,
    pub ny: usize,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundingLevel {
    pub pressure_hpa: f32,
    pub height_m: f32,
    pub temperature_c: f32,
    pub dewpoint_c: f32,
    pub wind_u_ms: f32,
    pub wind_v_ms: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundingProfile {
    pub station_id: String,
    pub levels: Vec<SoundingLevel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub station_id: String,
    pub latitude: f64,
    pub longitude: f64,
    pub temperature_c: f32,
    pub dewpoint_c: f32,
    pub wind_speed_ms: f32,
}

