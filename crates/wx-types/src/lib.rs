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
        finite_min_max(&self.values)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelAxis {
    pub kind: LevelMetadata,
    pub levels: Vec<LevelMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeAxis {
    pub cycles: Vec<DateTime<Utc>>,
    pub valid_times: Vec<DateTime<Utc>>,
    pub forecast_hours: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field3D {
    pub metadata: FieldMetadata,
    pub grid: GridSpec,
    pub level_axis: LevelAxis,
    pub values: Vec<f32>,
}

impl Field3D {
    pub fn nz(&self) -> usize {
        self.level_axis.levels.len()
    }

    pub fn expected_len(&self) -> usize {
        self.nz() * self.grid.nx * self.grid.ny
    }

    pub fn finite_min_max(&self) -> Option<(f32, f32)> {
        finite_min_max(&self.values)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldBundle {
    pub source: SourceMetadata,
    pub run: RunMetadata,
    pub valid: ValidTimeMetadata,
    pub grid: GridSpec,
    pub fields_2d: Vec<Field2D>,
    pub fields_3d: Vec<Field3D>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeVolume {
    pub product: String,
    pub time_axis: TimeAxis,
    pub bundle: FieldBundle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field2DSummary {
    pub short_name: String,
    pub parameter: String,
    pub level: String,
    pub units: String,
    pub nx: usize,
    pub ny: usize,
    pub finite_min: Option<f32>,
    pub finite_max: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field3DSummary {
    pub short_name: String,
    pub parameter: String,
    pub units: String,
    pub level_count: usize,
    pub levels: Vec<String>,
    pub nx: usize,
    pub ny: usize,
    pub finite_min: Option<f32>,
    pub finite_max: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldBundleSummary {
    pub source: SourceMetadata,
    pub run: RunMetadata,
    pub valid: ValidTimeMetadata,
    pub grid: GridSpec,
    pub fields_2d: Vec<Field2DSummary>,
    pub fields_3d: Vec<Field3DSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestedField {
    pub variable: String,
    pub level: String,
    pub forecast: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveJobSpec {
    pub model: String,
    pub product: String,
    pub start_cycle: DateTime<Utc>,
    pub end_cycle: DateTime<Utc>,
    pub cycle_step_hours: u16,
    pub forecast_hour: u16,
    pub selections: Vec<RequestedField>,
    pub output_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistedStoreKind {
    ZarrV2Directory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedArrayDescriptor {
    pub name: String,
    pub dimensions: Vec<String>,
    pub shape: Vec<usize>,
    pub chunks: Vec<usize>,
    pub dtype: String,
    pub units: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedStoreDescriptor {
    pub kind: PersistedStoreKind,
    pub root_path: String,
    pub source: SourceMetadata,
    pub run: RunMetadata,
    pub valid: ValidTimeMetadata,
    pub grid: GridSpec,
    pub arrays: Vec<PersistedArrayDescriptor>,
    pub compression_level: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchiveCycleState {
    Planned,
    Downloaded,
    Decoded,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveCycleRecord {
    pub cycle: DateTime<Utc>,
    pub source_name: Option<String>,
    pub source_grib_url: Option<String>,
    pub source_idx_url: Option<String>,
    pub subset_path: Option<String>,
    pub staged_manifest_path: Option<String>,
    pub decoded_summary_path: Option<String>,
    pub persisted_store_path: Option<String>,
    pub state: ArchiveCycleState,
    pub message_count: usize,
    pub field_count_2d: usize,
    pub field_count_3d: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveRunManifest {
    pub job: ArchiveJobSpec,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub cycles: Vec<ArchiveCycleRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSlice {
    pub source: SourceMetadata,
    pub run: RunMetadata,
    pub valid: ValidTimeMetadata,
    pub bundle_summary: FieldBundleSummary,
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
    pub grid_x: Option<usize>,
    pub grid_y: Option<usize>,
    pub valid_time: Option<DateTime<Utc>>,
    pub levels: Vec<SoundingLevel>,
}

fn finite_min_max(values: &[f32]) -> Option<(f32, f32)> {
    values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(None, |acc, value| match acc {
            None => Some((value, value)),
            Some((min_value, max_value)) => Some((min_value.min(value), max_value.max(value))),
        })
}
