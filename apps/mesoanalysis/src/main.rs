use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeZone, Utc};
use image::Rgba;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wx_fetch::{HrrrSelectionRequest, HrrrSubsetRequest, plan_hrrr_fixture_subset};
use wx_grib::{build_field_bundle, decode_selected_messages, summarize_field_bundle};
use wx_grid::{
    advection_field, anomaly_field, dewpoint_celsius_field, divergence_field, field_stats,
    lapse_rate_field, potential_temperature_field, pressure_level_frontogenesis_field,
    relative_humidity_field, smooth_n_point_field, temperature_celsius_field, thickness_field,
    vorticity_field, wind_speed_field,
};
use wx_render::{MapContourSpec, MapOverlaySpec, MapWindBarbSpec, render_field_to_map_png};
use wx_types::{ArchiveCycleState, ArchiveRunManifest, Field2D, FieldBundle};
use wx_zarr::{ZarrWriteConfig, read_field_bundle_from_zarr, write_field_bundle_to_zarr};

const MS_TO_KTS: f64 = 1.943_844_49;
const CONTOUR_COLOR: Rgba<u8> = Rgba([34, 39, 46, 255]);
const BARB_COLOR: Rgba<u8> = Rgba([26, 31, 37, 255]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProductKind {
    NativeField {
        short_name: &'static str,
        level: &'static str,
    },
    TemperatureCelsius {
        level: &'static str,
    },
    DewpointCelsius {
        level: &'static str,
    },
    RelativeHumidity {
        level: &'static str,
    },
    WindSpeed {
        level: &'static str,
    },
    Vorticity {
        level: &'static str,
        smooth_nine_point: bool,
    },
    Divergence {
        level: &'static str,
    },
    TemperatureAdvection {
        level: &'static str,
    },
    Theta {
        level: &'static str,
    },
    Frontogenesis {
        level: &'static str,
    },
    HeightAnomaly {
        level: &'static str,
    },
    Thickness {
        lower_level: &'static str,
        upper_level: &'static str,
    },
    LapseRate {
        lower_level: &'static str,
        upper_level: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MesoProductSpec {
    id: String,
    description: String,
    palette: String,
    kind: ProductKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum MesoCycleState {
    Pending,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RenderedProductRecord {
    product_id: String,
    png_path: String,
    value_min: f32,
    value_max: f32,
    mean_value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MesoCycleRecord {
    cycle: DateTime<Utc>,
    input_store_path: Option<String>,
    output_store_path: Option<String>,
    bundle_summary_path: Option<String>,
    rendered_products: Vec<RenderedProductRecord>,
    generated_fields: usize,
    state: MesoCycleState,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct MesoRunManifest {
    source_archive_manifest_path: String,
    output_root: String,
    products: Vec<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    cycles: Vec<MesoCycleRecord>,
}

#[derive(Debug, Clone)]
struct DerivedProduct {
    spec: MesoProductSpec,
    field: Field2D,
}

#[derive(Debug, Clone)]
struct CycleProducts {
    cycle: DateTime<Utc>,
    output_store_path: String,
    bundle_summary_path: String,
    rendered_products: Vec<RenderedProductRecord>,
    generated_fields: usize,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(|value| value.as_str()).unwrap_or("status");

    match command {
        "status" => print_status(),
        "catalog" => print_catalog(),
        "demo" => run_demo()?,
        "run" => run_batch(&args[1..])?,
        "resume" => run_resume(&args[1..])?,
        _ => print_usage(),
    }

    Ok(())
}

fn print_status() {
    println!("mesoanalysis: real HRRR batch product driver over archive manifests and Zarr stores");
    println!(
        "mesoanalysis: consumes persisted wx-zarr FieldBundle stores and writes derived Zarr + PNG outputs"
    );
    println!(
        "mesoanalysis: current registry spans surface, upper-air, thickness, lapse-rate, anomaly, and classic dynamics products"
    );
    println!(
        "mesoanalysis: products render with wrf-rust-plots-style palettes plus height contours and wind barbs where the bundle supports them"
    );
    println!(
        "mesoanalysis: demo still supports the checked-in offline HRRR pressure/surface fixtures"
    );
}

fn print_catalog() {
    println!("available mesoanalysis products:");
    for spec in product_registry() {
        let requires = required_fields(&spec.kind)
            .iter()
            .map(|(short_name, level)| format!("{short_name}:{level}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  {}: {} (requires {})",
            spec.id, spec.description, requires
        );
    }
}

fn run_demo() -> Result<()> {
    let cycle = Utc
        .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
        .single()
        .expect("valid fixture cycle");
    let fixture_root = repo_root().join("tests/fixtures");

    let surface_fragment = fixture_root.join("hrrr_demo_surface_fragment.grib2");
    let surface_idx_text =
        std::fs::read_to_string(fixture_root.join("hrrr_demo_surface_fragment.idx"))
            .context("failed to read surface fixture idx")?;
    let surface_plan = plan_hrrr_fixture_subset(
        &surface_demo_request(cycle),
        &surface_idx_text,
        std::fs::metadata(&surface_fragment)
            .context("failed to stat surface fixture")?
            .len(),
    )?;

    let pressure_fragment = fixture_root.join("hrrr_demo_pressure_fragment.grib2");
    let pressure_idx_text =
        std::fs::read_to_string(fixture_root.join("hrrr_demo_pressure_fragment.idx"))
            .context("failed to read pressure fixture idx")?;
    let pressure_plan = plan_hrrr_fixture_subset(
        &pressure_demo_request(cycle),
        &pressure_idx_text,
        std::fs::metadata(&pressure_fragment)
            .context("failed to stat pressure fixture")?
            .len(),
    )?;

    let surface_bundle =
        build_field_bundle(&decode_selected_messages(&surface_fragment, &surface_plan)?)?;
    let pressure_bundle = build_field_bundle(&decode_selected_messages(
        &pressure_fragment,
        &pressure_plan,
    )?)?;
    let bundle = merge_field_bundles(surface_bundle, pressure_bundle, "fixture_mesoanalysis")?;
    let selected_products = demo_product_ids();
    let output_root = repo_root().join("target/demo/mesoanalysis");
    let cycle_products = process_bundle_cycle(&bundle, cycle, &output_root, &selected_products)?;

    println!(
        "surface_source_range_origin={} surface_source_grib_url={}",
        surface_plan.byte_range_origin.label(),
        surface_plan.source_grib_url
    );
    println!(
        "pressure_source_range_origin={} pressure_source_grib_url={}",
        pressure_plan.byte_range_origin.label(),
        pressure_plan.source_grib_url
    );
    println!(
        "demo_cycle={} generated_fields={} derived_store={}",
        cycle.format("%Y%m%d%H"),
        cycle_products.generated_fields,
        cycle_products.output_store_path
    );
    for product in &cycle_products.rendered_products {
        println!(
            "{} range={:.8}..{:.8} mean={:.8} png={}",
            product.product_id,
            product.value_min,
            product.value_max,
            product.mean_value,
            product.png_path
        );
    }
    println!("bundle_summary={}", cycle_products.bundle_summary_path);

    Ok(())
}

fn run_batch(args: &[String]) -> Result<()> {
    let archive_manifest_path = args
        .first()
        .map(PathBuf::from)
        .context("run requires a path to archive_manifest.json")?;
    let output_root = args
        .get(1)
        .map(PathBuf::from)
        .context("run requires an output root")?;
    let products = parse_product_ids(&args[2..])?;
    let source_manifest: ArchiveRunManifest = read_json(&archive_manifest_path)?;

    let mut manifest = build_meso_run_manifest(
        &archive_manifest_path,
        &output_root,
        &products,
        &source_manifest,
    );
    execute_mesoanalysis(&mut manifest)?;
    let manifest_path = output_root.join("mesoanalysis_manifest.json");
    write_json(&manifest_path, &manifest)?;
    println!("mesoanalysis_manifest={}", manifest_path.display());
    Ok(())
}

fn demo_product_ids() -> Vec<String> {
    [
        "surface_gust",
        "10m_wind_speed",
        "2m_temperature",
        "2m_relative_humidity",
        "850mb_temperature_height_winds",
        "850mb_wind_speed_height",
        "850mb_smoothed_vorticity_height_winds",
        "850mb_frontogenesis_height_winds",
        "700mb_temperature_advection_height_winds",
        "500mb_height_anomaly",
        "thickness_1000_500mb",
        "lapse_rate_700_500mb",
    ]
    .iter()
    .map(|value| (*value).to_string())
    .collect()
}

fn run_resume(args: &[String]) -> Result<()> {
    let manifest_path = args
        .first()
        .map(PathBuf::from)
        .context("resume requires a path to mesoanalysis_manifest.json")?;
    let mut manifest: MesoRunManifest = read_json(&manifest_path)?;
    execute_mesoanalysis(&mut manifest)?;
    write_json(&manifest_path, &manifest)?;
    println!("mesoanalysis_manifest={}", manifest_path.display());
    Ok(())
}

fn execute_mesoanalysis(manifest: &mut MesoRunManifest) -> Result<()> {
    let output_root = PathBuf::from(&manifest.output_root);
    std::fs::create_dir_all(&output_root)?;
    std::fs::create_dir_all(output_root.join("zarr"))?;
    std::fs::create_dir_all(output_root.join("summaries"))?;
    std::fs::create_dir_all(output_root.join("png"))?;

    let pending_cycles: Vec<_> = manifest
        .cycles
        .iter()
        .filter(|record| record.state == MesoCycleState::Pending)
        .cloned()
        .collect();

    let products = manifest.products.clone();
    let results: Vec<_> = pending_cycles
        .into_par_iter()
        .map(|record| {
            let cycle = record.cycle;
            let result = process_meso_cycle(&output_root, &products, &record);
            (cycle, result)
        })
        .collect();

    for (cycle, result) in results {
        let record = manifest
            .cycles
            .iter_mut()
            .find(|record| record.cycle == cycle)
            .with_context(|| format!("missing mesoanalysis cycle record for {cycle}"))?;
        match result {
            Ok(cycle_products) => {
                record.output_store_path = Some(cycle_products.output_store_path);
                record.bundle_summary_path = Some(cycle_products.bundle_summary_path);
                record.rendered_products = cycle_products.rendered_products;
                record.generated_fields = cycle_products.generated_fields;
                record.state = MesoCycleState::Completed;
                record.error = None;
                println!(
                    "mesoanalysis cycle={} generated_fields={} output_store={}",
                    cycle_products.cycle.format("%Y%m%d%H"),
                    cycle_products.generated_fields,
                    record.output_store_path.as_deref().unwrap_or("<missing>")
                );
            }
            Err(error) => {
                record.state = MesoCycleState::Failed;
                record.error = Some(error.to_string());
                println!(
                    "mesoanalysis cycle={} failed: {}",
                    cycle.format("%Y%m%d%H"),
                    error
                );
            }
        }
        manifest.updated_at = Utc::now();
    }

    Ok(())
}

fn process_meso_cycle(
    output_root: &Path,
    products: &[String],
    record: &MesoCycleRecord,
) -> Result<CycleProducts> {
    let input_store_path = record
        .input_store_path
        .as_ref()
        .context("mesoanalysis cycle is missing an input store path")?;
    let bundle = read_field_bundle_from_zarr(Path::new(input_store_path))
        .with_context(|| format!("failed to read Zarr bundle {}", input_store_path))?;
    process_bundle_cycle(&bundle, record.cycle, output_root, products)
}

fn process_bundle_cycle(
    bundle: &FieldBundle,
    cycle: DateTime<Utc>,
    output_root: &Path,
    product_ids: &[String],
) -> Result<CycleProducts> {
    let derived_products = compute_products(bundle, product_ids)?;
    if derived_products.is_empty() {
        bail!(
            "no mesoanalysis products were generated for cycle {}",
            cycle.format("%Y%m%d%H")
        );
    }

    let derived_bundle = build_derived_bundle(bundle, &derived_products);
    let store_descriptor = write_field_bundle_to_zarr(
        &derived_bundle,
        &output_root.join("zarr"),
        &ZarrWriteConfig::default(),
    )?;
    let summary = summarize_field_bundle(&derived_bundle);
    let summary_path = output_root.join("summaries").join(format!(
        "mesoanalysis_{}_f{:02}_bundle.json",
        cycle.format("%Y%m%d%H"),
        bundle.run.forecast_hour
    ));
    write_json(&summary_path, &summary)?;

    let png_root = output_root.join("png").join(format!(
        "{}_f{:02}",
        cycle.format("%Y%m%d%H"),
        bundle.run.forecast_hour
    ));
    let rendered_products = derived_products
        .iter()
        .map(|product| {
            let stats = field_stats(&product.field).with_context(|| {
                format!(
                    "derived field {} contained no finite values",
                    product.spec.id
                )
            })?;
            let png_path = png_root.join(format!("{}.png", product.spec.id));
            let overlay = render_field_to_map_png(
                &product.field,
                &build_overlay_spec(bundle, product)?,
                &png_path,
            )?;
            Ok(RenderedProductRecord {
                product_id: product.spec.id.clone(),
                png_path: overlay.output_path.to_string_lossy().to_string(),
                value_min: overlay.value_min,
                value_max: overlay.value_max,
                mean_value: stats.mean_value,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(CycleProducts {
        cycle,
        output_store_path: store_descriptor.root_path,
        bundle_summary_path: summary_path.to_string_lossy().to_string(),
        rendered_products,
        generated_fields: derived_bundle.fields_2d.len(),
    })
}

fn compute_products(bundle: &FieldBundle, product_ids: &[String]) -> Result<Vec<DerivedProduct>> {
    let specs = select_product_specs(product_ids)?;
    let mut products = Vec::new();
    for spec in specs {
        if !bundle_supports_product(bundle, &spec) {
            continue;
        }
        match compute_product(bundle, spec.clone()) {
            Ok(product) => products.push(product),
            Err(error) => eprintln!("skipping {}: {}", spec.id, error),
        }
    }
    Ok(products)
}

fn compute_product(bundle: &FieldBundle, spec: MesoProductSpec) -> Result<DerivedProduct> {
    let field = match spec.kind {
        ProductKind::NativeField { short_name, level } => {
            select_field(bundle, short_name, level)?.clone()
        }
        ProductKind::TemperatureCelsius { level } => {
            temperature_celsius_field(select_field(bundle, "TMP", level)?)?
        }
        ProductKind::DewpointCelsius { level } => {
            dewpoint_celsius_field(select_field(bundle, "DPT", level)?)?
        }
        ProductKind::RelativeHumidity { level } => relative_humidity_field(
            select_field(bundle, "TMP", level)?,
            select_field(bundle, "DPT", level)?,
        )?,
        ProductKind::WindSpeed { level } => wind_speed_field(
            select_field(bundle, "UGRD", level)?,
            select_field(bundle, "VGRD", level)?,
        )?,
        ProductKind::Vorticity {
            level,
            smooth_nine_point,
        } => {
            let vorticity = vorticity_field(
                select_field(bundle, "UGRD", level)?,
                select_field(bundle, "VGRD", level)?,
            )?;
            if smooth_nine_point {
                smooth_n_point_field(&vorticity, 9, 1)?
            } else {
                vorticity
            }
        }
        ProductKind::Divergence { level } => divergence_field(
            select_field(bundle, "UGRD", level)?,
            select_field(bundle, "VGRD", level)?,
        )?,
        ProductKind::TemperatureAdvection { level } => advection_field(
            select_field(bundle, "TMP", level)?,
            select_field(bundle, "UGRD", level)?,
            select_field(bundle, "VGRD", level)?,
        )?,
        ProductKind::Theta { level } => {
            potential_temperature_field(select_field(bundle, "TMP", level)?)?
        }
        ProductKind::Frontogenesis { level } => pressure_level_frontogenesis_field(
            select_field(bundle, "TMP", level)?,
            select_field(bundle, "UGRD", level)?,
            select_field(bundle, "VGRD", level)?,
        )?,
        ProductKind::HeightAnomaly { level } => anomaly_field(select_field(bundle, "HGT", level)?)?,
        ProductKind::Thickness {
            lower_level,
            upper_level,
        } => thickness_field(
            select_field(bundle, "HGT", lower_level)?,
            select_field(bundle, "HGT", upper_level)?,
        )?,
        ProductKind::LapseRate {
            lower_level,
            upper_level,
        } => lapse_rate_field(
            select_field(bundle, "TMP", lower_level)?,
            select_field(bundle, "TMP", upper_level)?,
            select_field(bundle, "HGT", lower_level)?,
            select_field(bundle, "HGT", upper_level)?,
        )?,
    };

    Ok(DerivedProduct { spec, field })
}

fn build_derived_bundle(source_bundle: &FieldBundle, products: &[DerivedProduct]) -> FieldBundle {
    let mut source = source_bundle.source.clone();
    source.product = format!("{}_mesoanalysis", source_bundle.source.product);

    FieldBundle {
        source,
        run: source_bundle.run.clone(),
        valid: source_bundle.valid.clone(),
        grid: source_bundle.grid.clone(),
        fields_2d: products
            .iter()
            .map(|product| product.field.clone())
            .collect(),
        fields_3d: Vec::new(),
    }
}

fn merge_field_bundles(
    mut primary: FieldBundle,
    secondary: FieldBundle,
    product_name: &str,
) -> Result<FieldBundle> {
    if primary.grid != secondary.grid {
        bail!("cannot merge field bundles with different grids");
    }
    if primary.run != secondary.run {
        bail!("cannot merge field bundles with different run metadata");
    }
    if primary.valid != secondary.valid {
        bail!("cannot merge field bundles with different valid-time metadata");
    }
    if primary.source.provider != secondary.source.provider
        || primary.source.model != secondary.source.model
    {
        bail!("cannot merge field bundles from different source provenance");
    }

    primary.source.product = product_name.to_string();
    primary.fields_2d.extend(secondary.fields_2d);
    primary.fields_3d.extend(secondary.fields_3d);
    Ok(primary)
}

fn product_registry() -> Vec<MesoProductSpec> {
    let mut registry = vec![
        MesoProductSpec {
            id: "surface_gust".to_string(),
            description: "Surface gust".to_string(),
            palette: "winds".to_string(),
            kind: ProductKind::NativeField {
                short_name: "GUST",
                level: "surface",
            },
        },
        MesoProductSpec {
            id: "10m_wind_speed".to_string(),
            description: "10 m wind speed".to_string(),
            palette: "winds".to_string(),
            kind: ProductKind::WindSpeed {
                level: "10 m above ground",
            },
        },
        MesoProductSpec {
            id: "2m_temperature".to_string(),
            description: "2 m temperature".to_string(),
            palette: "temperature".to_string(),
            kind: ProductKind::TemperatureCelsius {
                level: "2 m above ground",
            },
        },
        MesoProductSpec {
            id: "2m_dewpoint".to_string(),
            description: "2 m dewpoint".to_string(),
            palette: "dewpoint".to_string(),
            kind: ProductKind::DewpointCelsius {
                level: "2 m above ground",
            },
        },
        MesoProductSpec {
            id: "2m_relative_humidity".to_string(),
            description: "2 m relative humidity".to_string(),
            palette: "rh".to_string(),
            kind: ProductKind::RelativeHumidity {
                level: "2 m above ground",
            },
        },
    ];

    for level in [
        "1000 mb", "925 mb", "850 mb", "700 mb", "500 mb", "400 mb", "300 mb",
    ] {
        let slug = level_slug(level);
        registry.extend([
            MesoProductSpec {
                id: format!("{slug}_wind_speed_height"),
                description: format!("{level} wind speed / height / winds"),
                palette: "winds".to_string(),
                kind: ProductKind::WindSpeed { level },
            },
            MesoProductSpec {
                id: format!("{slug}_temperature_height_winds"),
                description: format!("{level} temperature / height / winds"),
                palette: "temperature".to_string(),
                kind: ProductKind::TemperatureCelsius { level },
            },
            MesoProductSpec {
                id: format!("{slug}_dewpoint_height_winds"),
                description: format!("{level} dewpoint / height / winds"),
                palette: "dewpoint".to_string(),
                kind: ProductKind::DewpointCelsius { level },
            },
            MesoProductSpec {
                id: format!("{slug}_rh_height_winds"),
                description: format!("{level} relative humidity / height / winds"),
                palette: "rh".to_string(),
                kind: ProductKind::RelativeHumidity { level },
            },
            MesoProductSpec {
                id: format!("{slug}_theta_height_winds"),
                description: format!("{level} potential temperature / height / winds"),
                palette: "theta".to_string(),
                kind: ProductKind::Theta { level },
            },
            MesoProductSpec {
                id: format!("{slug}_vorticity_height_winds"),
                description: format!("{level} relative vorticity / height / winds"),
                palette: "vorticity".to_string(),
                kind: ProductKind::Vorticity {
                    level,
                    smooth_nine_point: false,
                },
            },
            MesoProductSpec {
                id: format!("{slug}_smoothed_vorticity_height_winds"),
                description: format!("{level} smoothed relative vorticity / height / winds"),
                palette: "vorticity".to_string(),
                kind: ProductKind::Vorticity {
                    level,
                    smooth_nine_point: true,
                },
            },
            MesoProductSpec {
                id: format!("{slug}_divergence_height_winds"),
                description: format!("{level} divergence / height / winds"),
                palette: "divergence".to_string(),
                kind: ProductKind::Divergence { level },
            },
            MesoProductSpec {
                id: format!("{slug}_temperature_advection_height_winds"),
                description: format!("{level} temperature advection / height / winds"),
                palette: "advection".to_string(),
                kind: ProductKind::TemperatureAdvection { level },
            },
        ]);
    }

    for level in ["925 mb", "850 mb", "700 mb", "500 mb"] {
        let slug = level_slug(level);
        registry.push(MesoProductSpec {
            id: format!("{slug}_frontogenesis_height_winds"),
            description: format!("{level} theta frontogenesis / height / winds"),
            palette: "frontogenesis".to_string(),
            kind: ProductKind::Frontogenesis { level },
        });
    }

    registry.extend([
        MesoProductSpec {
            id: "500mb_height_anomaly".to_string(),
            description: "500 mb geopotential height anomaly".to_string(),
            palette: "height_anomaly".to_string(),
            kind: ProductKind::HeightAnomaly { level: "500 mb" },
        },
        MesoProductSpec {
            id: "thickness_1000_850mb".to_string(),
            description: "1000-850 mb thickness".to_string(),
            palette: "temperature".to_string(),
            kind: ProductKind::Thickness {
                lower_level: "1000 mb",
                upper_level: "850 mb",
            },
        },
        MesoProductSpec {
            id: "thickness_1000_500mb".to_string(),
            description: "1000-500 mb thickness".to_string(),
            palette: "temperature".to_string(),
            kind: ProductKind::Thickness {
                lower_level: "1000 mb",
                upper_level: "500 mb",
            },
        },
        MesoProductSpec {
            id: "thickness_850_500mb".to_string(),
            description: "850-500 mb thickness".to_string(),
            palette: "temperature".to_string(),
            kind: ProductKind::Thickness {
                lower_level: "850 mb",
                upper_level: "500 mb",
            },
        },
        MesoProductSpec {
            id: "thickness_700_500mb".to_string(),
            description: "700-500 mb thickness".to_string(),
            palette: "temperature".to_string(),
            kind: ProductKind::Thickness {
                lower_level: "700 mb",
                upper_level: "500 mb",
            },
        },
        MesoProductSpec {
            id: "lapse_rate_1000_850mb".to_string(),
            description: "1000-850 mb lapse rate".to_string(),
            palette: "temperature".to_string(),
            kind: ProductKind::LapseRate {
                lower_level: "1000 mb",
                upper_level: "850 mb",
            },
        },
        MesoProductSpec {
            id: "lapse_rate_850_500mb".to_string(),
            description: "850-500 mb lapse rate".to_string(),
            palette: "lapse_rate".to_string(),
            kind: ProductKind::LapseRate {
                lower_level: "850 mb",
                upper_level: "500 mb",
            },
        },
        MesoProductSpec {
            id: "lapse_rate_700_500mb".to_string(),
            description: "700-500 mb lapse rate".to_string(),
            palette: "lapse_rate".to_string(),
            kind: ProductKind::LapseRate {
                lower_level: "700 mb",
                upper_level: "500 mb",
            },
        },
    ]);

    registry
}

fn level_slug(level: &str) -> String {
    level.replace(" mb", "mb").replace(' ', "_").to_lowercase()
}

fn required_fields(kind: &ProductKind) -> Vec<(&'static str, &'static str)> {
    match kind {
        ProductKind::NativeField { short_name, level } => vec![(*short_name, *level)],
        ProductKind::TemperatureCelsius { level } => {
            if *level == "2 m above ground" {
                vec![
                    ("TMP", level),
                    ("UGRD", "10 m above ground"),
                    ("VGRD", "10 m above ground"),
                ]
            } else {
                vec![
                    ("TMP", level),
                    ("HGT", level),
                    ("UGRD", level),
                    ("VGRD", level),
                ]
            }
        }
        ProductKind::DewpointCelsius { level } => {
            if *level == "2 m above ground" {
                vec![
                    ("DPT", level),
                    ("UGRD", "10 m above ground"),
                    ("VGRD", "10 m above ground"),
                ]
            } else {
                vec![
                    ("DPT", level),
                    ("HGT", level),
                    ("UGRD", level),
                    ("VGRD", level),
                ]
            }
        }
        ProductKind::RelativeHumidity { level } => {
            if *level == "2 m above ground" {
                vec![
                    ("TMP", level),
                    ("DPT", level),
                    ("UGRD", "10 m above ground"),
                    ("VGRD", "10 m above ground"),
                ]
            } else {
                vec![
                    ("TMP", level),
                    ("DPT", level),
                    ("HGT", level),
                    ("UGRD", level),
                    ("VGRD", level),
                ]
            }
        }
        ProductKind::WindSpeed { level } => {
            if *level == "10 m above ground" {
                vec![("UGRD", level), ("VGRD", level)]
            } else {
                vec![("UGRD", level), ("VGRD", level), ("HGT", level)]
            }
        }
        ProductKind::Vorticity { level, .. }
        | ProductKind::Divergence { level }
        | ProductKind::TemperatureAdvection { level }
        | ProductKind::Theta { level }
        | ProductKind::Frontogenesis { level } => match kind {
            ProductKind::TemperatureAdvection { .. }
            | ProductKind::Theta { .. }
            | ProductKind::Frontogenesis { .. } => {
                vec![
                    ("TMP", level),
                    ("UGRD", level),
                    ("VGRD", level),
                    ("HGT", level),
                ]
            }
            _ => vec![("UGRD", level), ("VGRD", level), ("HGT", level)],
        },
        ProductKind::HeightAnomaly { level } => vec![("HGT", level)],
        ProductKind::Thickness {
            lower_level,
            upper_level,
        } => {
            vec![("HGT", lower_level), ("HGT", upper_level)]
        }
        ProductKind::LapseRate {
            lower_level,
            upper_level,
        } => vec![
            ("TMP", lower_level),
            ("TMP", upper_level),
            ("HGT", lower_level),
            ("HGT", upper_level),
        ],
    }
}

fn bundle_supports_product(bundle: &FieldBundle, spec: &MesoProductSpec) -> bool {
    required_fields(&spec.kind)
        .iter()
        .all(|(short_name, level)| field_present(bundle, short_name, level))
}

fn field_present(bundle: &FieldBundle, short_name: &str, level: &str) -> bool {
    bundle.fields_2d.iter().any(|field| {
        field.metadata.short_name == short_name
            && field.metadata.level.description.eq_ignore_ascii_case(level)
    })
}

fn build_overlay_spec(bundle: &FieldBundle, product: &DerivedProduct) -> Result<MapOverlaySpec> {
    Ok(MapOverlaySpec {
        palette: product.spec.palette.clone(),
        value_range: None,
        levels: levels_for_kind(&product.spec.kind),
        tick_step: tick_step_for_kind(&product.spec.kind),
        title: Some(product.spec.description.clone()),
        subtitle: Some(format!(
            "{} | {} | valid {}",
            bundle.source.model.to_uppercase(),
            bundle.run.cycle.format("%Y-%m-%d %HZ"),
            bundle.valid.valid_time.format("%Y-%m-%d %HZ")
        )),
        subtitle_right: Some(format!(
            "{} | f{:02}",
            product.field.metadata.level.description, bundle.run.forecast_hour
        )),
        colorbar_label: Some(product.field.metadata.units.clone()),
        markers: Vec::new(),
        contours: contour_specs_for_kind(bundle, &product.spec.kind)?,
        barbs: wind_barb_specs_for_kind(bundle, &product.spec.kind)?,
    })
}

fn upper_air_level(kind: &ProductKind) -> Option<&'static str> {
    match kind {
        ProductKind::TemperatureCelsius { level }
        | ProductKind::DewpointCelsius { level }
        | ProductKind::RelativeHumidity { level }
        | ProductKind::WindSpeed { level }
        | ProductKind::Vorticity { level, .. }
        | ProductKind::Divergence { level }
        | ProductKind::TemperatureAdvection { level }
        | ProductKind::Theta { level }
        | ProductKind::Frontogenesis { level }
        | ProductKind::HeightAnomaly { level }
            if level.ends_with("mb") =>
        {
            Some(level)
        }
        _ => None,
    }
}

fn contour_specs_for_kind(bundle: &FieldBundle, kind: &ProductKind) -> Result<Vec<MapContourSpec>> {
    let Some(level) = upper_air_level(kind) else {
        return Ok(Vec::new());
    };
    Ok(vec![MapContourSpec {
        field: select_field(bundle, "HGT", level)?.clone(),
        levels: height_contour_levels(level),
        color: CONTOUR_COLOR,
        width: 2,
        labels: true,
        label_scale: 0.1,
        show_extrema: false,
    }])
}

fn wind_barb_specs_for_kind(
    bundle: &FieldBundle,
    kind: &ProductKind,
) -> Result<Vec<MapWindBarbSpec>> {
    if let Some(level) = upper_air_level(kind) {
        return Ok(vec![MapWindBarbSpec {
            u_field: select_field(bundle, "UGRD", level)?.clone(),
            v_field: select_field(bundle, "VGRD", level)?.clone(),
            stride_x: 32,
            stride_y: 28,
            color: BARB_COLOR,
            width: 2,
            length_px: 20.0,
            speed_scale: MS_TO_KTS,
        }]);
    }

    match kind {
        ProductKind::TemperatureCelsius { level }
        | ProductKind::DewpointCelsius { level }
        | ProductKind::RelativeHumidity { level }
            if *level == "2 m above ground" =>
        {
            Ok(vec![MapWindBarbSpec {
                u_field: select_field(bundle, "UGRD", "10 m above ground")?.clone(),
                v_field: select_field(bundle, "VGRD", "10 m above ground")?.clone(),
                stride_x: 40,
                stride_y: 34,
                color: BARB_COLOR,
                width: 2,
                length_px: 18.0,
                speed_scale: MS_TO_KTS,
            }])
        }
        ProductKind::WindSpeed { level } if *level == "10 m above ground" => {
            Ok(vec![MapWindBarbSpec {
                u_field: select_field(bundle, "UGRD", "10 m above ground")?.clone(),
                v_field: select_field(bundle, "VGRD", "10 m above ground")?.clone(),
                stride_x: 40,
                stride_y: 34,
                color: BARB_COLOR,
                width: 2,
                length_px: 18.0,
                speed_scale: MS_TO_KTS,
            }])
        }
        _ => Ok(Vec::new()),
    }
}

fn levels_for_kind(kind: &ProductKind) -> Option<Vec<f64>> {
    match kind {
        ProductKind::NativeField {
            short_name: "GUST", ..
        } => Some(inclusive_levels(0.0, 80.0, 2.5)),
        ProductKind::WindSpeed { .. } => Some(inclusive_levels(0.0, 120.0, 5.0)),
        ProductKind::TemperatureCelsius { level } => Some(temperature_levels_for_pressure(level)),
        ProductKind::DewpointCelsius { level } => Some(dewpoint_levels_for_pressure(level)),
        ProductKind::RelativeHumidity { .. } => Some(inclusive_levels(0.0, 100.0, 5.0)),
        ProductKind::Theta { .. } => Some(inclusive_levels(250.0, 390.0, 2.0)),
        ProductKind::HeightAnomaly { .. } => Some(inclusive_levels(-600.0, 600.0, 25.0)),
        ProductKind::Thickness {
            lower_level,
            upper_level,
        } => Some(thickness_levels(lower_level, upper_level)),
        ProductKind::LapseRate { .. } => Some(inclusive_levels(2.0, 10.0, 0.5)),
        _ => None,
    }
}

fn tick_step_for_kind(kind: &ProductKind) -> Option<f64> {
    match kind {
        ProductKind::NativeField {
            short_name: "GUST", ..
        }
        | ProductKind::WindSpeed { .. } => Some(10.0),
        ProductKind::TemperatureCelsius { .. }
        | ProductKind::DewpointCelsius { .. }
        | ProductKind::Theta { .. } => Some(10.0),
        ProductKind::RelativeHumidity { .. } => Some(10.0),
        ProductKind::HeightAnomaly { .. } => Some(100.0),
        ProductKind::Thickness { .. } => Some(60.0),
        ProductKind::LapseRate { .. } => Some(1.0),
        _ => None,
    }
}

fn inclusive_levels(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let mut levels = Vec::new();
    let mut value = start;
    while value <= stop + step * 0.01 {
        levels.push(value);
        value += step;
    }
    levels
}

fn temperature_levels_for_pressure(level: &str) -> Vec<f64> {
    match level {
        "2 m above ground" => inclusive_levels(-30.0, 45.0, 2.0),
        "1000 mb" => inclusive_levels(-20.0, 35.0, 2.0),
        "925 mb" => inclusive_levels(-20.0, 30.0, 2.0),
        "850 mb" => inclusive_levels(-30.0, 25.0, 2.0),
        "700 mb" => inclusive_levels(-40.0, 15.0, 2.0),
        "500 mb" => inclusive_levels(-50.0, 0.0, 2.0),
        "400 mb" => inclusive_levels(-60.0, -5.0, 2.0),
        "300 mb" => inclusive_levels(-75.0, -20.0, 2.0),
        _ => inclusive_levels(-60.0, 40.0, 2.0),
    }
}

fn dewpoint_levels_for_pressure(level: &str) -> Vec<f64> {
    match level {
        "2 m above ground" => inclusive_levels(-35.0, 30.0, 2.0),
        "1000 mb" => inclusive_levels(-30.0, 25.0, 2.0),
        "925 mb" => inclusive_levels(-30.0, 20.0, 2.0),
        "850 mb" => inclusive_levels(-40.0, 15.0, 2.0),
        "700 mb" => inclusive_levels(-55.0, 5.0, 2.0),
        "500 mb" => inclusive_levels(-70.0, -5.0, 2.0),
        "400 mb" => inclusive_levels(-80.0, -10.0, 2.0),
        "300 mb" => inclusive_levels(-90.0, -20.0, 2.0),
        _ => inclusive_levels(-80.0, 20.0, 2.0),
    }
}

fn thickness_levels(lower_level: &str, upper_level: &str) -> Vec<f64> {
    match (lower_level, upper_level) {
        ("1000 mb", "850 mb") => inclusive_levels(1000.0, 1800.0, 30.0),
        ("1000 mb", "500 mb") => inclusive_levels(4800.0, 6000.0, 60.0),
        ("850 mb", "500 mb") => inclusive_levels(3000.0, 4200.0, 30.0),
        ("700 mb", "500 mb") => inclusive_levels(1500.0, 3000.0, 30.0),
        _ => inclusive_levels(0.0, 6000.0, 60.0),
    }
}

fn height_contour_levels(level: &str) -> Vec<f64> {
    match level {
        "1000 mb" => inclusive_levels(-100.0, 500.0, 30.0),
        "925 mb" => inclusive_levels(300.0, 1500.0, 30.0),
        "850 mb" => inclusive_levels(0.0, 2000.0, 30.0),
        "700 mb" => inclusive_levels(1000.0, 4000.0, 30.0),
        "500 mb" => inclusive_levels(4500.0, 6500.0, 30.0),
        "400 mb" => inclusive_levels(6500.0, 8000.0, 40.0),
        "300 mb" => inclusive_levels(8000.0, 10000.0, 40.0),
        _ => inclusive_levels(0.0, 10000.0, 50.0),
    }
}

fn select_product_specs(product_ids: &[String]) -> Result<Vec<MesoProductSpec>> {
    let registry = product_registry();
    if product_ids.is_empty() {
        return Ok(registry);
    }

    product_ids
        .iter()
        .map(|product_id| {
            registry
                .iter()
                .find(|spec| spec.id.eq_ignore_ascii_case(product_id))
                .cloned()
                .with_context(|| format!("unknown mesoanalysis product {}", product_id))
        })
        .collect()
}

fn build_meso_run_manifest(
    archive_manifest_path: &Path,
    output_root: &Path,
    products: &[String],
    archive_manifest: &ArchiveRunManifest,
) -> MesoRunManifest {
    let cycles = archive_manifest
        .cycles
        .iter()
        .map(|cycle| {
            let ready = cycle.state == ArchiveCycleState::Completed
                && cycle
                    .persisted_store_path
                    .as_ref()
                    .is_some_and(|path| Path::new(path).exists());
            let (state, error) = if ready {
                (MesoCycleState::Pending, None)
            } else {
                (
                    MesoCycleState::Failed,
                    Some(
                        "archive cycle does not have a completed persisted store for mesoanalysis"
                            .to_string(),
                    ),
                )
            };
            MesoCycleRecord {
                cycle: cycle.cycle,
                input_store_path: cycle.persisted_store_path.clone(),
                output_store_path: None,
                bundle_summary_path: None,
                rendered_products: Vec::new(),
                generated_fields: 0,
                state,
                error,
            }
        })
        .collect();

    MesoRunManifest {
        source_archive_manifest_path: archive_manifest_path.to_string_lossy().to_string(),
        output_root: output_root.to_string_lossy().to_string(),
        products: if products.is_empty() {
            product_registry().into_iter().map(|spec| spec.id).collect()
        } else {
            products.to_vec()
        },
        created_at: Utc::now(),
        updated_at: Utc::now(),
        cycles,
    }
}

fn parse_product_ids(args: &[String]) -> Result<Vec<String>> {
    let registry = product_registry();
    if args.is_empty() {
        return Ok(registry.into_iter().map(|spec| spec.id).collect());
    }

    let mut products = Vec::new();
    for arg in args {
        if arg.eq_ignore_ascii_case("all") {
            return Ok(product_registry().into_iter().map(|spec| spec.id).collect());
        }
        if !registry
            .iter()
            .any(|spec| spec.id.eq_ignore_ascii_case(arg))
        {
            bail!("unknown mesoanalysis product {}", arg);
        }
        products.push(arg.to_string());
    }

    Ok(products)
}

fn surface_demo_request(cycle: chrono::DateTime<Utc>) -> HrrrSubsetRequest {
    HrrrSubsetRequest {
        cycle,
        forecast_hour: 0,
        product: "sfc".to_string(),
        selections: vec![
            selection("GUST", "surface"),
            selection("TMP", "2 m above ground"),
            selection("DPT", "2 m above ground"),
            selection("UGRD", "10 m above ground"),
            selection("VGRD", "10 m above ground"),
        ],
    }
}

fn pressure_demo_request(cycle: chrono::DateTime<Utc>) -> HrrrSubsetRequest {
    let mut selections = Vec::new();
    for level in [
        "1000 mb", "925 mb", "850 mb", "700 mb", "500 mb", "400 mb", "300 mb",
    ] {
        for variable in ["HGT", "TMP", "DPT", "UGRD", "VGRD"] {
            selections.push(selection(variable, level));
        }
    }
    HrrrSubsetRequest {
        cycle,
        forecast_hour: 0,
        product: "prs".to_string(),
        selections,
    }
}

fn selection(variable: &'static str, level: &'static str) -> HrrrSelectionRequest {
    HrrrSelectionRequest {
        variable: variable.to_string(),
        level: level.to_string(),
        forecast: Some("anl".to_string()),
    }
}

fn select_field<'a>(bundle: &'a FieldBundle, variable: &str, level: &str) -> Result<&'a Field2D> {
    bundle
        .fields_2d
        .iter()
        .find(|field| {
            field.metadata.short_name == variable
                && field.metadata.level.description.eq_ignore_ascii_case(level)
        })
        .with_context(|| format!("missing {variable} at {level} in mesoanalysis bundle"))
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn print_usage() {
    println!("usage:");
    println!("  cargo run -p mesoanalysis-app -- status");
    println!("  cargo run -p mesoanalysis-app -- catalog");
    println!("  cargo run -p mesoanalysis-app -- demo");
    println!(
        "  cargo run -p mesoanalysis-app -- run <archive_manifest.json> <output_root> [product...]"
    );
    println!("  cargo run -p mesoanalysis-app -- resume <mesoanalysis_manifest.json>");
    println!("products:");
    for spec in product_registry() {
        println!("  {}", spec.id);
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace layout is stable")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_bundle() -> FieldBundle {
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid fixture cycle");
        let fixture_root = repo_root().join("tests/fixtures");
        let surface_fragment = fixture_root.join("hrrr_demo_surface_fragment.grib2");
        let pressure_fragment = fixture_root.join("hrrr_demo_pressure_fragment.grib2");
        let surface_idx =
            std::fs::read_to_string(fixture_root.join("hrrr_demo_surface_fragment.idx"))
                .expect("surface idx should be readable");
        let pressure_idx =
            std::fs::read_to_string(fixture_root.join("hrrr_demo_pressure_fragment.idx"))
                .expect("pressure idx should be readable");
        let surface_plan = plan_hrrr_fixture_subset(
            &surface_demo_request(cycle),
            &surface_idx,
            std::fs::metadata(&surface_fragment)
                .expect("surface fixture should exist")
                .len(),
        )
        .expect("surface plan should succeed");
        let pressure_plan = plan_hrrr_fixture_subset(
            &pressure_demo_request(cycle),
            &pressure_idx,
            std::fs::metadata(&pressure_fragment)
                .expect("pressure fixture should exist")
                .len(),
        )
        .expect("pressure plan should succeed");
        let surface_bundle = build_field_bundle(
            &decode_selected_messages(&surface_fragment, &surface_plan)
                .expect("surface decode succeeds"),
        )
        .expect("surface bundle should build");
        let pressure_bundle = build_field_bundle(
            &decode_selected_messages(&pressure_fragment, &pressure_plan)
                .expect("pressure decode succeeds"),
        )
        .expect("pressure bundle should build");
        merge_field_bundles(surface_bundle, pressure_bundle, "fixture_mesoanalysis")
            .expect("fixture bundles should merge")
    }

    #[test]
    fn product_registry_accepts_all_keyword() {
        let products = parse_product_ids(&["all".to_string()]).expect("all should work");
        assert_eq!(products.len(), product_registry().len());
    }

    #[test]
    fn fixture_products_are_finite() {
        let bundle = fixture_bundle();
        let products = compute_products(
            &bundle,
            &[
                "surface_gust".to_string(),
                "10m_wind_speed".to_string(),
                "850mb_temperature_height_winds".to_string(),
                "850mb_smoothed_vorticity_height_winds".to_string(),
                "850mb_frontogenesis_height_winds".to_string(),
                "500mb_height_anomaly".to_string(),
                "thickness_1000_500mb".to_string(),
                "lapse_rate_850_500mb".to_string(),
            ],
        )
        .expect("product compute should succeed");

        assert_eq!(products.len(), 8);
        for product in products {
            let stats = field_stats(&product.field).expect("derived field should be finite");
            assert!(stats.min_value.is_finite());
            assert!(stats.max_value.is_finite());
        }
    }

    #[test]
    fn overlay_spec_adds_upper_air_layers() {
        let bundle = fixture_bundle();
        let product = compute_product(
            &bundle,
            product_registry()
                .into_iter()
                .find(|spec| spec.id == "850mb_temperature_height_winds")
                .expect("registry product should exist"),
        )
        .expect("compute should succeed");
        let overlay = build_overlay_spec(&bundle, &product).expect("overlay should build");

        assert!(!overlay.contours.is_empty());
        assert!(!overlay.barbs.is_empty());
    }

    #[test]
    fn derived_bundle_uses_mesoanalysis_product_namespace() {
        let bundle = fixture_bundle();
        let products = compute_products(&bundle, &["850mb_divergence_height_winds".to_string()])
            .expect("single product should compute");
        let derived = build_derived_bundle(&bundle, &products);

        assert!(derived.source.product.ends_with("_mesoanalysis"));
        assert_eq!(derived.fields_2d.len(), 1);
        assert!(derived.fields_3d.is_empty());
    }
}
