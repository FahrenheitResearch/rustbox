use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProjectionKind {
    LatitudeLongitude,
    LambertConformal { latin1: f64, latin2: f64, lov: f64 },
    Mercator { lad: f64 },
    PolarStereographic { lad: f64, lov: f64 },
    Unknown { template: u16 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordinateMetadata {
    pub lat1: f64,
    pub lon1: f64,
    pub lat2: f64,
    pub lon2: f64,
    pub dx: f64,
    pub dy: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridSpec {
    pub nx: usize,
    pub ny: usize,
    pub projection: ProjectionKind,
    pub coordinates: CoordinateMetadata,
    pub scan_mode: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceMetadata {
    pub provider: String,
    pub model: String,
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunMetadata {
    pub cycle: DateTime<Utc>,
    pub forecast_hour: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidTimeMetadata {
    pub reference_time: DateTime<Utc>,
    pub valid_time: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelMetadata {
    pub code: u8,
    pub description: String,
    pub value: Option<f64>,
    pub units: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldMetadata {
    pub short_name: String,
    pub parameter: String,
    pub units: String,
    pub level: LevelMetadata,
    pub source: SourceMetadata,
    pub run: RunMetadata,
    pub valid: ValidTimeMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field2D {
    pub metadata: FieldMetadata,
    pub grid: GridSpec,
    pub values: Vec<f32>,
}

impl Field2D {
    pub fn expected_len(&self) -> usize {
        self.grid.nx * self.grid.ny
    }

    pub fn finite_min_max(&self) -> Option<(f32, f32)> {
        self.values
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .fold(None, |acc, value| match acc {
                None => Some((value, value)),
                Some((min_value, max_value)) => Some((min_value.min(value), max_value.max(value))),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundingLevel {
    pub pressure_hpa: f64,
    pub height_m: f64,
    pub temperature_c: f64,
    pub dewpoint_c: f64,
    pub wind_direction_deg: f64,
    pub wind_speed_kts: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundingProfile {
    pub station_id: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub valid_time: Option<DateTime<Utc>>,
    pub levels: Vec<SoundingLevel>,
}
