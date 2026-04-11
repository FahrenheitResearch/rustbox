use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeZone, Utc};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wx_fetch::{HrrrSelectionRequest, HrrrSubsetRequest, plan_hrrr_fixture_subset};
use wx_grib::{build_field_bundle, decode_selected_messages, summarize_field_bundle};
use wx_grid::{
    advection_field, divergence_field, field_stats, pressure_level_frontogenesis_field,
    smooth_n_point_field, vorticity_field,
};
use wx_render::{MapOverlaySpec, render_field_to_map_png};
use wx_types::{ArchiveCycleState, ArchiveRunManifest, Field2D, FieldBundle};
use wx_zarr::{ZarrWriteConfig, read_field_bundle_from_zarr, write_field_bundle_to_zarr};

const DEMO_LEVEL: &str = "850 mb";

#[derive(Debug, Clone, Copy)]
struct MesoProductSpec {
    id: &'static str,
    description: &'static str,
    palette: &'static str,
    required_fields: &'static [(&'static str, &'static str)],
}

const PRODUCT_SMOOTHED_VORTICITY_850MB: MesoProductSpec = MesoProductSpec {
    id: "smoothed_vorticity_850mb",
    description: "850 mb relative vorticity with 9-point smoothing",
    palette: "vorticity",
    required_fields: &[("UGRD", DEMO_LEVEL), ("VGRD", DEMO_LEVEL)],
};

const PRODUCT_DIVERGENCE_850MB: MesoProductSpec = MesoProductSpec {
    id: "divergence_850mb",
    description: "850 mb horizontal divergence",
    palette: "divergence",
    required_fields: &[("UGRD", DEMO_LEVEL), ("VGRD", DEMO_LEVEL)],
};

const PRODUCT_TEMPERATURE_ADVECTION_850MB: MesoProductSpec = MesoProductSpec {
    id: "temperature_advection_850mb",
    description: "850 mb temperature advection",
    palette: "advection",
    required_fields: &[
        ("TMP", DEMO_LEVEL),
        ("UGRD", DEMO_LEVEL),
        ("VGRD", DEMO_LEVEL),
    ],
};

const PRODUCT_FRONTOGENESIS_850MB: MesoProductSpec = MesoProductSpec {
    id: "frontogenesis_850mb",
    description: "850 mb theta-based Petterssen frontogenesis",
    palette: "frontogenesis",
    required_fields: &[
        ("TMP", DEMO_LEVEL),
        ("UGRD", DEMO_LEVEL),
        ("VGRD", DEMO_LEVEL),
    ],
};

const PRODUCT_REGISTRY: &[MesoProductSpec] = &[
    PRODUCT_SMOOTHED_VORTICITY_850MB,
    PRODUCT_DIVERGENCE_850MB,
    PRODUCT_TEMPERATURE_ADVECTION_850MB,
    PRODUCT_FRONTOGENESIS_850MB,
];

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
        "mesoanalysis: current registry includes 850 mb smoothed vorticity, divergence, temperature advection, and theta frontogenesis"
    );
    println!(
        "mesoanalysis: demo still supports the checked-in offline 850 mb HRRR pressure fixture"
    );
}

fn print_catalog() {
    println!("available mesoanalysis products:");
    for spec in PRODUCT_REGISTRY {
        let requires = spec
            .required_fields
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
    let fragment = fixture_root.join("hrrr_demo_pressure_fragment.grib2");
    let idx_text = std::fs::read_to_string(fixture_root.join("hrrr_demo_pressure_fragment.idx"))
        .context("failed to read pressure fixture idx")?;
    let plan = plan_hrrr_fixture_subset(
        &pressure_demo_request(cycle),
        &idx_text,
        std::fs::metadata(&fragment)
            .context("failed to stat pressure fixture")?
            .len(),
    )?;

    let decoded = decode_selected_messages(&fragment, &plan)?;
    let bundle = build_field_bundle(&decoded)?;
    let selected_products: Vec<String> = PRODUCT_REGISTRY
        .iter()
        .map(|spec| spec.id.to_string())
        .collect();
    let output_root = repo_root().join("target/demo/mesoanalysis");
    let cycle_products = process_bundle_cycle(&bundle, cycle, &output_root, &selected_products)?;
    let height = select_field(&bundle, "HGT", DEMO_LEVEL)
        .context("mesoanalysis demo bundle was missing HGT 850 mb")?;
    let height_stats = field_stats(height).context("height field contained no finite values")?;

    println!(
        "source_range_origin={} pressure_source_grib_url={}",
        plan.byte_range_origin.label(),
        plan.source_grib_url
    );
    println!(
        "level={} grid={}x{} height_range_gpm={:.1}..{:.1}",
        DEMO_LEVEL, bundle.grid.nx, bundle.grid.ny, height_stats.min_value, height_stats.max_value
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
    println!("derived_store={}", cycle_products.output_store_path);
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
    products: &[String],
) -> Result<CycleProducts> {
    let derived_products = compute_products(bundle, products)?;
    if derived_products.is_empty() {
        bail!(
            "no mesoanalysis products were generated for cycle {}",
            cycle
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
                &MapOverlaySpec {
                    palette: product.spec.palette.to_string(),
                    value_range: None,
                    title: Some(product.spec.description.to_string()),
                    subtitle: Some(format!(
                        "{} | {} | valid {}",
                        bundle.source.model.to_uppercase(),
                        bundle.run.cycle.format("%Y-%m-%d %HZ"),
                        bundle.valid.valid_time.format("%Y-%m-%d %HZ")
                    )),
                    colorbar_label: Some(product.field.metadata.units.clone()),
                    markers: Vec::new(),
                },
                &png_path,
            )?;
            Ok(RenderedProductRecord {
                product_id: product.spec.id.to_string(),
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
    specs
        .into_iter()
        .map(|spec| compute_product(bundle, spec))
        .collect()
}

fn compute_product(bundle: &FieldBundle, spec: MesoProductSpec) -> Result<DerivedProduct> {
    let field = match spec.id {
        "smoothed_vorticity_850mb" => {
            let u_wind = select_field(bundle, "UGRD", DEMO_LEVEL)?;
            let v_wind = select_field(bundle, "VGRD", DEMO_LEVEL)?;
            let vorticity = vorticity_field(u_wind, v_wind)?;
            smooth_n_point_field(&vorticity, 9, 1)?
        }
        "divergence_850mb" => {
            let u_wind = select_field(bundle, "UGRD", DEMO_LEVEL)?;
            let v_wind = select_field(bundle, "VGRD", DEMO_LEVEL)?;
            divergence_field(u_wind, v_wind)?
        }
        "temperature_advection_850mb" => {
            let temperature = select_field(bundle, "TMP", DEMO_LEVEL)?;
            let u_wind = select_field(bundle, "UGRD", DEMO_LEVEL)?;
            let v_wind = select_field(bundle, "VGRD", DEMO_LEVEL)?;
            advection_field(temperature, u_wind, v_wind)?
        }
        "frontogenesis_850mb" => {
            let temperature = select_field(bundle, "TMP", DEMO_LEVEL)?;
            let u_wind = select_field(bundle, "UGRD", DEMO_LEVEL)?;
            let v_wind = select_field(bundle, "VGRD", DEMO_LEVEL)?;
            pressure_level_frontogenesis_field(temperature, u_wind, v_wind)?
        }
        other => bail!("unsupported mesoanalysis product {}", other),
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

fn select_product_specs(product_ids: &[String]) -> Result<Vec<MesoProductSpec>> {
    if product_ids.is_empty() {
        return Ok(PRODUCT_REGISTRY.to_vec());
    }

    product_ids
        .iter()
        .map(|product_id| {
            PRODUCT_REGISTRY
                .iter()
                .find(|spec| spec.id.eq_ignore_ascii_case(product_id))
                .copied()
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
            PRODUCT_REGISTRY
                .iter()
                .map(|spec| spec.id.to_string())
                .collect()
        } else {
            products.to_vec()
        },
        created_at: Utc::now(),
        updated_at: Utc::now(),
        cycles,
    }
}

fn parse_product_ids(args: &[String]) -> Result<Vec<String>> {
    if args.is_empty() {
        return Ok(PRODUCT_REGISTRY
            .iter()
            .map(|spec| spec.id.to_string())
            .collect());
    }

    let mut products = Vec::new();
    for arg in args {
        if arg.eq_ignore_ascii_case("all") {
            return Ok(PRODUCT_REGISTRY
                .iter()
                .map(|spec| spec.id.to_string())
                .collect());
        }
        if !PRODUCT_REGISTRY
            .iter()
            .any(|spec| spec.id.eq_ignore_ascii_case(arg))
        {
            bail!("unknown mesoanalysis product {}", arg);
        }
        products.push(arg.to_string());
    }

    Ok(products)
}

fn pressure_demo_request(cycle: chrono::DateTime<Utc>) -> HrrrSubsetRequest {
    HrrrSubsetRequest {
        cycle,
        forecast_hour: 0,
        product: "prs".to_string(),
        selections: vec![
            HrrrSelectionRequest {
                variable: "HGT".to_string(),
                level: DEMO_LEVEL.to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "TMP".to_string(),
                level: DEMO_LEVEL.to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "UGRD".to_string(),
                level: DEMO_LEVEL.to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "VGRD".to_string(),
                level: DEMO_LEVEL.to_string(),
                forecast: Some("anl".to_string()),
            },
        ],
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
    for spec in PRODUCT_REGISTRY {
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
        let fragment = fixture_root.join("hrrr_demo_pressure_fragment.grib2");
        let idx_text =
            std::fs::read_to_string(fixture_root.join("hrrr_demo_pressure_fragment.idx"))
                .expect("pressure fixture idx should be readable");
        let plan = plan_hrrr_fixture_subset(
            &pressure_demo_request(cycle),
            &idx_text,
            std::fs::metadata(&fragment)
                .expect("pressure fixture should exist")
                .len(),
        )
        .expect("fixture plan should succeed");
        let decoded = decode_selected_messages(&fragment, &plan).expect("fixture decode succeeds");
        build_field_bundle(&decoded).expect("fixture bundle should build")
    }

    #[test]
    fn product_registry_accepts_all_keyword() {
        let products = parse_product_ids(&["all".to_string()]).expect("all should work");
        assert_eq!(products.len(), PRODUCT_REGISTRY.len());
    }

    #[test]
    fn fixture_products_are_finite() {
        let bundle = fixture_bundle();
        let products = compute_products(
            &bundle,
            &[
                "smoothed_vorticity_850mb".to_string(),
                "divergence_850mb".to_string(),
                "temperature_advection_850mb".to_string(),
                "frontogenesis_850mb".to_string(),
            ],
        )
        .expect("product compute should succeed");

        assert_eq!(products.len(), 4);
        for product in products {
            let stats = field_stats(&product.field).expect("derived field should be finite");
            assert!(stats.min_value.is_finite());
            assert!(stats.max_value.is_finite());
        }
    }

    #[test]
    fn derived_bundle_uses_mesoanalysis_product_namespace() {
        let bundle = fixture_bundle();
        let products = compute_products(&bundle, &["divergence_850mb".to_string()])
            .expect("single product should compute");
        let derived = build_derived_bundle(&bundle, &products);

        assert_eq!(derived.source.product, "prs_mesoanalysis");
        assert_eq!(derived.fields_2d.len(), 1);
        assert!(derived.fields_3d.is_empty());
    }
}
