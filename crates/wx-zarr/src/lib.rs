use anyhow::{Context, Result};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use wx_types::{
    Field2D, Field3D, FieldBundle, PersistedArrayDescriptor, PersistedStoreDescriptor,
    PersistedStoreKind, ProjectionKind,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZarrWriteConfig {
    pub chunk_xy: [usize; 2],
    pub chunk_zyx: [usize; 3],
    pub compression_level: u32,
}

impl Default for ZarrWriteConfig {
    fn default() -> Self {
        Self {
            chunk_xy: [256, 256],
            chunk_zyx: [1, 256, 256],
            compression_level: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ZarrArray {
    zarr_format: u8,
    shape: Vec<usize>,
    chunks: Vec<usize>,
    dtype: String,
    compressor: Option<ZarrCompressor>,
    fill_value: serde_json::Value,
    order: String,
    filters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ZarrCompressor {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ZarrGroup {
    zarr_format: u8,
}

struct ZarrStore {
    root: PathBuf,
}

impl ZarrStore {
    fn create(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        let group = ZarrGroup { zarr_format: 2 };
        fs::write(
            root.join(".zgroup"),
            serde_json::to_vec_pretty(&group).context("failed to serialize .zgroup")?,
        )?;
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    fn create_array(
        &self,
        name: &str,
        shape: &[usize],
        chunks: &[usize],
        dtype: &str,
        fill_value: serde_json::Value,
        compression_level: u32,
    ) -> Result<PathBuf> {
        let array_dir = self.root.join(name);
        fs::create_dir_all(&array_dir)?;

        let compressor = if compression_level > 0 {
            Some(ZarrCompressor {
                id: "zlib".to_string(),
                level: Some(compression_level),
            })
        } else {
            None
        };

        let zarray = ZarrArray {
            zarr_format: 2,
            shape: shape.to_vec(),
            chunks: chunks.to_vec(),
            dtype: dtype.to_string(),
            compressor,
            fill_value,
            order: "C".to_string(),
            filters: None,
        };

        fs::write(
            array_dir.join(".zarray"),
            serde_json::to_vec_pretty(&zarray).context("failed to serialize .zarray")?,
        )?;
        Ok(array_dir)
    }

    fn write_attrs(&self, name: &str, attrs: &serde_json::Value) -> Result<()> {
        let target = if name.is_empty() {
            self.root.clone()
        } else {
            self.root.join(name)
        };
        fs::create_dir_all(&target)?;
        fs::write(
            target.join(".zattrs"),
            serde_json::to_vec_pretty(attrs).context("failed to serialize .zattrs")?,
        )?;
        Ok(())
    }

    fn write_chunk_f32(
        &self,
        array_name: &str,
        chunk_key: &str,
        data: &[f32],
        compression_level: u32,
    ) -> Result<()> {
        let raw_bytes: Vec<u8> = data.iter().flat_map(|value| value.to_le_bytes()).collect();
        let bytes = maybe_compress(&raw_bytes, compression_level)?;
        fs::write(self.root.join(array_name).join(chunk_key), bytes)?;
        Ok(())
    }

    fn write_chunk_f64(
        &self,
        array_name: &str,
        chunk_key: &str,
        data: &[f64],
        compression_level: u32,
    ) -> Result<()> {
        let raw_bytes: Vec<u8> = data.iter().flat_map(|value| value.to_le_bytes()).collect();
        let bytes = maybe_compress(&raw_bytes, compression_level)?;
        fs::write(self.root.join(array_name).join(chunk_key), bytes)?;
        Ok(())
    }
}

pub fn write_field_bundle_to_zarr(
    bundle: &FieldBundle,
    output_root: &Path,
    config: &ZarrWriteConfig,
) -> Result<PersistedStoreDescriptor> {
    let store_path = output_root
        .join(&bundle.source.model)
        .join(&bundle.source.product)
        .join(format!(
            "{}_f{:02}.zarr",
            bundle.run.cycle.format("%Y%m%d%H"),
            bundle.run.forecast_hour
        ));
    let store = ZarrStore::create(&store_path)?;

    let root_attrs = serde_json::json!({
        "Conventions": "CF-1.8",
        "source": bundle.source,
        "run": {
            "cycle": bundle.run.cycle.to_rfc3339(),
            "forecast_hour": bundle.run.forecast_hour,
        },
        "valid": {
            "reference_time": bundle.valid.reference_time.to_rfc3339(),
            "valid_time": bundle.valid.valid_time.to_rfc3339(),
        },
        "grid": {
            "nx": bundle.grid.nx,
            "ny": bundle.grid.ny,
            "projection": bundle.grid.projection,
            "coordinates": bundle.grid.coordinates,
            "scan_mode": bundle.grid.scan_mode,
        },
        "rustbox_store_kind": "field_bundle",
        "array_count_2d": bundle.fields_2d.len(),
        "array_count_3d": bundle.fields_3d.len(),
    });
    store.write_attrs("", &root_attrs)?;

    let mut arrays = Vec::new();
    write_core_coordinates(&store, bundle, config.compression_level, &mut arrays)?;

    for field in &bundle.fields_2d {
        arrays.push(write_field2d_array(&store, field, config)?);
    }
    for field in &bundle.fields_3d {
        arrays.push(write_field3d_array(&store, field, config)?);
        arrays.push(write_level_coordinate_array(
            &store,
            field,
            config.compression_level,
        )?);
    }

    Ok(PersistedStoreDescriptor {
        kind: PersistedStoreKind::ZarrV2Directory,
        root_path: store_path.to_string_lossy().to_string(),
        source: bundle.source.clone(),
        run: bundle.run.clone(),
        valid: bundle.valid.clone(),
        grid: bundle.grid.clone(),
        arrays,
        compression_level: config.compression_level,
    })
}

fn write_core_coordinates(
    store: &ZarrStore,
    bundle: &FieldBundle,
    _compression_level: u32,
    arrays: &mut Vec<PersistedArrayDescriptor>,
) -> Result<()> {
    let y_name = "coordinates/y";
    let x_name = "coordinates/x";
    let y_values: Vec<f64> = (0..bundle.grid.ny)
        .map(|index| index as f64 * bundle.grid.coordinates.dy)
        .collect();
    let x_values: Vec<f64> = (0..bundle.grid.nx)
        .map(|index| index as f64 * bundle.grid.coordinates.dx)
        .collect();

    store.create_array(
        y_name,
        &[bundle.grid.ny],
        &[bundle.grid.ny],
        "<f8",
        serde_json::json!("NaN"),
        0,
    )?;
    store.write_attrs(
        y_name,
        &serde_json::json!({
            "_ARRAY_DIMENSIONS": ["y"],
            "units": coordinate_units_for_projection(&bundle.grid.projection, true),
            "long_name": "y coordinate",
        }),
    )?;
    store.write_chunk_f64(y_name, "0", &y_values, 0)?;
    arrays.push(PersistedArrayDescriptor {
        name: y_name.to_string(),
        dimensions: vec!["y".to_string()],
        shape: vec![bundle.grid.ny],
        chunks: vec![bundle.grid.ny],
        dtype: "<f8".to_string(),
        units: coordinate_units_for_projection(&bundle.grid.projection, true).to_string(),
    });

    store.create_array(
        x_name,
        &[bundle.grid.nx],
        &[bundle.grid.nx],
        "<f8",
        serde_json::json!("NaN"),
        0,
    )?;
    store.write_attrs(
        x_name,
        &serde_json::json!({
            "_ARRAY_DIMENSIONS": ["x"],
            "units": coordinate_units_for_projection(&bundle.grid.projection, false),
            "long_name": "x coordinate",
        }),
    )?;
    store.write_chunk_f64(x_name, "0", &x_values, 0)?;
    arrays.push(PersistedArrayDescriptor {
        name: x_name.to_string(),
        dimensions: vec!["x".to_string()],
        shape: vec![bundle.grid.nx],
        chunks: vec![bundle.grid.nx],
        dtype: "<f8".to_string(),
        units: coordinate_units_for_projection(&bundle.grid.projection, false).to_string(),
    });

    if matches!(bundle.grid.projection, ProjectionKind::LatitudeLongitude) {
        let lat_name = "coordinates/latitude";
        let lon_name = "coordinates/longitude";
        let lats = linear_axis(
            bundle.grid.coordinates.lat1,
            bundle.grid.coordinates.lat2,
            bundle.grid.ny,
        );
        let lons = linear_axis(
            bundle.grid.coordinates.lon1,
            bundle.grid.coordinates.lon2,
            bundle.grid.nx,
        );
        store.create_array(
            lat_name,
            &[bundle.grid.ny],
            &[bundle.grid.ny],
            "<f8",
            serde_json::json!("NaN"),
            0,
        )?;
        store.write_attrs(
            lat_name,
            &serde_json::json!({
                "_ARRAY_DIMENSIONS": ["y"],
                "units": "degrees_north",
                "long_name": "latitude",
                "standard_name": "latitude",
            }),
        )?;
        store.write_chunk_f64(lat_name, "0", &lats, 0)?;
        arrays.push(PersistedArrayDescriptor {
            name: lat_name.to_string(),
            dimensions: vec!["y".to_string()],
            shape: vec![bundle.grid.ny],
            chunks: vec![bundle.grid.ny],
            dtype: "<f8".to_string(),
            units: "degrees_north".to_string(),
        });

        store.create_array(
            lon_name,
            &[bundle.grid.nx],
            &[bundle.grid.nx],
            "<f8",
            serde_json::json!("NaN"),
            0,
        )?;
        store.write_attrs(
            lon_name,
            &serde_json::json!({
                "_ARRAY_DIMENSIONS": ["x"],
                "units": "degrees_east",
                "long_name": "longitude",
                "standard_name": "longitude",
            }),
        )?;
        store.write_chunk_f64(lon_name, "0", &lons, 0)?;
        arrays.push(PersistedArrayDescriptor {
            name: lon_name.to_string(),
            dimensions: vec!["x".to_string()],
            shape: vec![bundle.grid.nx],
            chunks: vec![bundle.grid.nx],
            dtype: "<f8".to_string(),
            units: "degrees_east".to_string(),
        });
    }

    Ok(())
}

fn write_field2d_array(
    store: &ZarrStore,
    field: &Field2D,
    config: &ZarrWriteConfig,
) -> Result<PersistedArrayDescriptor> {
    let array_name = format!(
        "fields_2d/{}_{}",
        slugify(&field.metadata.short_name),
        slugify(&field.metadata.level.description)
    );
    let cy = config.chunk_xy[0].min(field.grid.ny.max(1));
    let cx = config.chunk_xy[1].min(field.grid.nx.max(1));
    store.create_array(
        &array_name,
        &[field.grid.ny, field.grid.nx],
        &[cy, cx],
        "<f4",
        serde_json::json!("NaN"),
        config.compression_level,
    )?;
    store.write_attrs(
        &array_name,
        &serde_json::json!({
            "_ARRAY_DIMENSIONS": ["y", "x"],
            "short_name": field.metadata.short_name,
            "parameter": field.metadata.parameter,
            "units": field.metadata.units,
            "level": field.metadata.level,
        }),
    )?;

    let n_chunks_y = field.grid.ny.div_ceil(cy);
    let n_chunks_x = field.grid.nx.div_ceil(cx);
    for chunk_y in 0..n_chunks_y {
        for chunk_x in 0..n_chunks_x {
            let y_start = chunk_y * cy;
            let x_start = chunk_x * cx;
            let y_end = (y_start + cy).min(field.grid.ny);
            let x_end = (x_start + cx).min(field.grid.nx);
            let mut chunk = vec![f32::NAN; cy * cx];
            for local_y in 0..(y_end - y_start) {
                let src_start = (y_start + local_y) * field.grid.nx + x_start;
                let src_end = src_start + (x_end - x_start);
                let dst_start = local_y * cx;
                chunk[dst_start..dst_start + (x_end - x_start)]
                    .copy_from_slice(&field.values[src_start..src_end]);
            }
            store.write_chunk_f32(
                &array_name,
                &format!("{}.{}", chunk_y, chunk_x),
                &chunk,
                config.compression_level,
            )?;
        }
    }

    Ok(PersistedArrayDescriptor {
        name: array_name,
        dimensions: vec!["y".to_string(), "x".to_string()],
        shape: vec![field.grid.ny, field.grid.nx],
        chunks: vec![cy, cx],
        dtype: "<f4".to_string(),
        units: field.metadata.units.clone(),
    })
}

fn write_field3d_array(
    store: &ZarrStore,
    field: &Field3D,
    config: &ZarrWriteConfig,
) -> Result<PersistedArrayDescriptor> {
    let array_name = format!("fields_3d/{}", slugify(&field.metadata.short_name));
    let cz = config.chunk_zyx[0].min(field.nz().max(1));
    let cy = config.chunk_zyx[1].min(field.grid.ny.max(1));
    let cx = config.chunk_zyx[2].min(field.grid.nx.max(1));
    store.create_array(
        &array_name,
        &[field.nz(), field.grid.ny, field.grid.nx],
        &[cz, cy, cx],
        "<f4",
        serde_json::json!("NaN"),
        config.compression_level,
    )?;
    store.write_attrs(
        &array_name,
        &serde_json::json!({
            "_ARRAY_DIMENSIONS": ["level", "y", "x"],
            "short_name": field.metadata.short_name,
            "parameter": field.metadata.parameter,
            "units": field.metadata.units,
            "level_axis": field.level_axis,
        }),
    )?;

    let ny = field.grid.ny;
    let nx = field.grid.nx;
    let n_chunks_z = field.nz().div_ceil(cz);
    let n_chunks_y = ny.div_ceil(cy);
    let n_chunks_x = nx.div_ceil(cx);
    for chunk_z in 0..n_chunks_z {
        for chunk_y in 0..n_chunks_y {
            for chunk_x in 0..n_chunks_x {
                let z_start = chunk_z * cz;
                let y_start = chunk_y * cy;
                let x_start = chunk_x * cx;
                let z_end = (z_start + cz).min(field.nz());
                let y_end = (y_start + cy).min(ny);
                let x_end = (x_start + cx).min(nx);
                let mut chunk = vec![f32::NAN; cz * cy * cx];

                for local_z in 0..(z_end - z_start) {
                    for local_y in 0..(y_end - y_start) {
                        let src_z = z_start + local_z;
                        let src_y = y_start + local_y;
                        let src_start = src_z * ny * nx + src_y * nx + x_start;
                        let src_end = src_start + (x_end - x_start);
                        let dst_start = local_z * cy * cx + local_y * cx;
                        chunk[dst_start..dst_start + (x_end - x_start)]
                            .copy_from_slice(&field.values[src_start..src_end]);
                    }
                }
                store.write_chunk_f32(
                    &array_name,
                    &format!("{}.{}.{}", chunk_z, chunk_y, chunk_x),
                    &chunk,
                    config.compression_level,
                )?;
            }
        }
    }

    Ok(PersistedArrayDescriptor {
        name: array_name,
        dimensions: vec!["level".to_string(), "y".to_string(), "x".to_string()],
        shape: vec![field.nz(), ny, nx],
        chunks: vec![cz, cy, cx],
        dtype: "<f4".to_string(),
        units: field.metadata.units.clone(),
    })
}

fn write_level_coordinate_array(
    store: &ZarrStore,
    field: &Field3D,
    _compression_level: u32,
) -> Result<PersistedArrayDescriptor> {
    let array_name = format!("coordinates/levels_{}", slugify(&field.metadata.short_name));
    let values: Vec<f64> = field
        .level_axis
        .levels
        .iter()
        .map(|level| level.value.unwrap_or(f64::NAN))
        .collect();
    let chunk = field.nz().max(1);
    store.create_array(
        &array_name,
        &[field.nz()],
        &[chunk],
        "<f8",
        serde_json::json!("NaN"),
        0,
    )?;
    store.write_attrs(
        &array_name,
        &serde_json::json!({
            "_ARRAY_DIMENSIONS": ["level"],
            "units": field.level_axis.kind.units,
            "long_name": "model level coordinate",
        }),
    )?;
    store.write_chunk_f64(&array_name, "0", &values, 0)?;
    Ok(PersistedArrayDescriptor {
        name: array_name,
        dimensions: vec!["level".to_string()],
        shape: vec![field.nz()],
        chunks: vec![chunk],
        dtype: "<f8".to_string(),
        units: field.level_axis.kind.units.clone(),
    })
}

fn coordinate_units_for_projection(projection: &ProjectionKind, is_y: bool) -> &'static str {
    match projection {
        ProjectionKind::LatitudeLongitude => {
            if is_y {
                "degrees_north"
            } else {
                "degrees_east"
            }
        }
        _ => "m",
    }
}

fn linear_axis(start: f64, end: f64, count: usize) -> Vec<f64> {
    match count {
        0 => Vec::new(),
        1 => vec![start],
        _ => {
            let step = (end - start) / (count as f64 - 1.0);
            (0..count)
                .map(|index| start + step * index as f64)
                .collect()
        }
    }
}

fn slugify(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn maybe_compress(data: &[u8], compression_level: u32) -> Result<Vec<u8>> {
    if compression_level == 0 {
        return Ok(data.to_vec());
    }

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(compression_level));
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use wx_types::{
        CoordinateMetadata, FieldMetadata, GridSpec, LevelAxis, LevelMetadata, ProjectionKind,
        RunMetadata, SourceMetadata, ValidTimeMetadata,
    };

    #[test]
    fn writer_creates_real_zarr_store_for_bundle() {
        let root = std::env::temp_dir().join(format!("rustbox-zarr-{}", std::process::id()));
        let bundle = sample_bundle();
        let descriptor =
            write_field_bundle_to_zarr(&bundle, &root, &ZarrWriteConfig::default()).expect("write");

        let store_root = PathBuf::from(&descriptor.root_path);
        assert!(store_root.join(".zgroup").exists());
        assert!(store_root.join("fields_2d/tmp_850_mb/.zarray").exists());
        assert!(store_root.join("fields_3d/tmp/.zarray").exists());
        assert!(store_root.join("coordinates/x/0").exists());
        assert!(
            descriptor
                .arrays
                .iter()
                .any(|array| array.name == "fields_3d/tmp")
        );

        let _ = fs::remove_dir_all(root);
    }

    fn sample_bundle() -> FieldBundle {
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        let grid = GridSpec {
            nx: 2,
            ny: 2,
            projection: ProjectionKind::LambertConformal {
                latin1: 38.5,
                latin2: 38.5,
                lov: 262.5,
            },
            coordinates: CoordinateMetadata {
                lat1: 21.138,
                lon1: 237.28,
                lat2: 47.12,
                lon2: 299.11,
                dx: 3000.0,
                dy: 3000.0,
            },
            scan_mode: 0,
        };
        let field_metadata = FieldMetadata {
            short_name: "TMP".to_string(),
            parameter: "Temperature".to_string(),
            units: "K".to_string(),
            level: LevelMetadata {
                code: 100,
                description: "850 mb".to_string(),
                value: Some(850.0),
                units: "hPa".to_string(),
            },
            source: SourceMetadata {
                provider: "hrrr-aws".to_string(),
                model: "hrrr".to_string(),
                product: "prs".to_string(),
            },
            run: RunMetadata {
                cycle,
                forecast_hour: 0,
            },
            valid: ValidTimeMetadata {
                reference_time: cycle,
                valid_time: cycle,
            },
        };
        let field_2d = Field2D {
            metadata: field_metadata.clone(),
            grid: grid.clone(),
            values: vec![280.0, 281.0, 282.0, 283.0],
        };
        let field_3d = Field3D {
            metadata: field_metadata,
            grid: grid.clone(),
            level_axis: LevelAxis {
                kind: LevelMetadata {
                    code: 100,
                    description: "isobaric".to_string(),
                    value: None,
                    units: "hPa".to_string(),
                },
                levels: vec![
                    LevelMetadata {
                        code: 100,
                        description: "850 mb".to_string(),
                        value: Some(850.0),
                        units: "hPa".to_string(),
                    },
                    LevelMetadata {
                        code: 100,
                        description: "700 mb".to_string(),
                        value: Some(700.0),
                        units: "hPa".to_string(),
                    },
                ],
            },
            values: vec![280.0, 281.0, 282.0, 283.0, 275.0, 276.0, 277.0, 278.0],
        };

        FieldBundle {
            source: SourceMetadata {
                provider: "hrrr-aws".to_string(),
                model: "hrrr".to_string(),
                product: "prs".to_string(),
            },
            run: RunMetadata {
                cycle,
                forecast_hour: 0,
            },
            valid: ValidTimeMetadata {
                reference_time: cycle,
                valid_time: cycle,
            },
            grid,
            fields_2d: vec![field_2d],
            fields_3d: vec![field_3d],
        }
    }
}
