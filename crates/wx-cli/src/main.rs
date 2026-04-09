use anyhow::{Context, Result, bail};
use chrono::{DateTime, TimeZone, Utc};
use std::path::{Path, PathBuf};
use wx_fetch::{
    DownloadClient, HrrrSelectionRequest, HrrrSubsetRequest, StagedSubset,
    build_hrrr_archive_requests, fetch_remote_idx_text, initial_archive_manifest,
    plan_hrrr_fixture_subset, plan_hrrr_remote_subset, probe_first_available_hrrr_source,
    stage_remote_subset_download, stage_subset_file_name,
};
use wx_grib::{
    build_hrrr_sounding_profile, decode_field_bundle, decode_selected_messages,
    summarize_field_bundle,
};
use wx_render::{OverlaySpec, render_field_to_png};
use wx_severe::compute_significant_tornado_parameter;
use wx_thermo::compute_parcel_diagnostics;
use wx_types::{ArchiveCycleState, ArchiveJobSpec, ArchiveRunManifest, RequestedField};

const DEMO_PRESSURE_LEVELS: [&str; 7] = [
    "1000 mb", "925 mb", "850 mb", "700 mb", "500 mb", "400 mb", "300 mb",
];
const DEMO_PROFILE_X: usize = 1_798;
const DEMO_PROFILE_Y: usize = 1_058;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(|value| value.as_str()).unwrap_or("status");

    match command {
        "status" => print_status(),
        "demo" => run_demo()?,
        "plan" => run_plan(&args[1..])?,
        "download" => run_download(&args[1..])?,
        "decode" => run_decode(&args[1..])?,
        "archive-run" => run_archive(&args[1..])?,
        "resume" => run_resume(&args[1..])?,
        _ => print_usage(),
    }

    Ok(())
}

fn print_status() {
    println!(
        "wx-fetch: real HRRR source probing, .idx subset planning, disk cache, staged subset download, and archive-cycle iteration"
    );
    println!(
        "wx-grib: real GRIB2 decode for scalar and multi-message HRRR bundles with stacked 3D field summaries"
    );
    println!(
        "wx-grib: real HRRR column extraction into SoundingProfile at a fixed fixture grid point"
    );
    println!("wx-thermo: real sharprs-derived SBCAPE/MLCAPE/MUCAPE/CIN diagnostics");
    println!(
        "wx-grid: real constant-spacing divergence/vorticity/theta-frontogenesis and 5/9-point smoothing"
    );
    println!(
        "wx-severe: real fixed-layer STP and exact-layer kinematics via a local sharprs compatibility fork"
    );
    println!("wx-render: real transparent PNG overlay writer");
    println!("wx-cli archive-core: plan/download/decode/archive-run/resume over HRRR cycle ranges");
    println!("wx-cuda: stub capability surface only");
    println!("wx-radar/wx-wrf/wx-zarr/wx-py: not implemented in this milestone");
}

fn run_demo() -> Result<()> {
    let cycle = Utc
        .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
        .single()
        .expect("valid fixture cycle");
    let fixture_root = repo_root().join("tests/fixtures");
    let (x, y) = (DEMO_PROFILE_X, DEMO_PROFILE_Y);

    let surface_fragment = fixture_root.join("hrrr_demo_surface_fragment.grib2");
    let surface_plan = plan_hrrr_fixture_subset(
        &surface_request(cycle),
        &std::fs::read_to_string(fixture_root.join("hrrr_demo_surface_fragment.idx"))?,
        std::fs::metadata(&surface_fragment)?.len(),
    )?;
    let pressure_fragment = fixture_root.join("hrrr_demo_pressure_fragment.grib2");
    let pressure_plan = plan_hrrr_fixture_subset(
        &pressure_request(cycle),
        &std::fs::read_to_string(fixture_root.join("hrrr_demo_pressure_fragment.idx"))?,
        std::fs::metadata(&pressure_fragment)?.len(),
    )?;

    let surface_messages = decode_selected_messages(&surface_fragment, &surface_plan)?;
    let pressure_messages = decode_selected_messages(&pressure_fragment, &pressure_plan)?;
    let sounding = build_hrrr_sounding_profile(&surface_messages, &pressure_messages, x, y)?;
    let parcel = compute_parcel_diagnostics(&sounding)?;
    let severe = compute_significant_tornado_parameter(&sounding, &parcel)?;
    let overlay_field = surface_messages
        .first()
        .map(|message| message.field.clone())
        .context("surface fixture decode did not return the requested gust field")?;

    let output_path = repo_root().join("target/demo/hrrr_gust_surface_overlay.png");
    let overlay = render_field_to_png(
        &overlay_field,
        &OverlaySpec {
            palette: "winds".to_string(),
            transparent_background: true,
            value_range: None,
        },
        &output_path,
    )?;

    let selection = &surface_plan.selections[0];
    let (field_min, field_max) = overlay_field
        .finite_min_max()
        .expect("decoded field should have data");

    println!(
        "selection_msg={} fragment_bytes={}..{} source_range_origin={}",
        selection.message_number,
        selection.start,
        selection.end_exclusive,
        surface_plan.byte_range_origin.label()
    );
    println!("surface_source_grib_url={}", surface_plan.source_grib_url);
    println!("pressure_source_grib_url={}", pressure_plan.source_grib_url);
    println!(
        "field={} level={} grid={}x{} range={:.2}..{:.2}",
        overlay_field.metadata.parameter,
        overlay_field.metadata.level.description,
        overlay_field.grid.nx,
        overlay_field.grid.ny,
        field_min,
        field_max
    );
    println!(
        "profile_point=x{} y{} levels={} sfc_p={:.1} top_p={:.1}",
        x,
        y,
        sounding.levels.len(),
        sounding
            .levels
            .first()
            .map(|level| level.pressure_hpa)
            .unwrap_or(0.0),
        sounding
            .levels
            .last()
            .map(|level| level.pressure_hpa)
            .unwrap_or(0.0)
    );
    println!(
        "sbcape={:.1} sbcin={:.1} mlcape={:.1} mlcin={:.1} mucape={:.1} mucin={:.1}",
        parcel.surface.cape_jkg,
        parcel.surface.cin_jkg,
        parcel.mixed_layer.cape_jkg,
        parcel.mixed_layer.cin_jkg,
        parcel.most_unstable.cape_jkg,
        parcel.most_unstable.cin_jkg
    );
    println!(
        "srh01={:.1} srh03={:.1} shear06={:.2} stp_fixed={:.2}",
        severe.kinematics.srh_01km_m2s2,
        severe.kinematics.srh_03km_m2s2,
        severe.kinematics.bulk_shear_06km_ms,
        severe.significant_tornado_parameter
    );
    println!("overlay_png={}", overlay.output_path.display());
    Ok(())
}

fn run_plan(args: &[String]) -> Result<()> {
    let parsed = parse_archive_args(args, false)?;
    let job = parsed.job("__plan_only__");
    let client = DownloadClient::with_cache_dir(None)?;
    let requests = build_hrrr_archive_requests(&job)?;

    println!(
        "planning cycles={} product={} forecast_hour={} step={}h selectors={}",
        requests.len(),
        job.product,
        job.forecast_hour,
        job.cycle_step_hours,
        job.selections.len()
    );

    for request in requests {
        let candidate = probe_first_available_hrrr_source(&client, &request)?;
        let idx_text = fetch_remote_idx_text(&client, &candidate)?;
        let plan = plan_hrrr_remote_subset(&request, &idx_text, &candidate)?;
        let total_bytes: u64 = plan
            .selections
            .iter()
            .map(|selection| selection.end_exclusive - selection.start)
            .sum();
        println!(
            "cycle={} source={} product={} f{:02} messages={} staged_bytes={} grib_url={}",
            request.cycle.format("%Y%m%d%H"),
            candidate.name,
            request.product,
            request.forecast_hour,
            plan.selections.len(),
            total_bytes,
            candidate.grib_url
        );
    }

    Ok(())
}

fn run_download(args: &[String]) -> Result<()> {
    let parsed = parse_archive_args(args, true)?;
    let output_root = parsed
        .output_root
        .clone()
        .context("download requires an output root")?;
    let job = parsed.job(output_root.display().to_string());
    let mut manifest = initial_archive_manifest(&job)?;
    let client = DownloadClient::with_cache_dir(None)?;
    let manifest_path = output_root.join("archive_manifest.json");

    execute_archive_job(&client, &job, &output_root, &mut manifest, false)?;
    write_json(&manifest_path, &manifest)?;
    println!("archive_manifest={}", manifest_path.display());
    Ok(())
}

fn run_archive(args: &[String]) -> Result<()> {
    let parsed = parse_archive_args(args, true)?;
    let output_root = parsed
        .output_root
        .clone()
        .context("archive-run requires an output root")?;
    let job = parsed.job(output_root.display().to_string());
    let mut manifest = initial_archive_manifest(&job)?;
    let client = DownloadClient::with_cache_dir(None)?;
    let manifest_path = output_root.join("archive_manifest.json");

    execute_archive_job(&client, &job, &output_root, &mut manifest, true)?;
    write_json(&manifest_path, &manifest)?;
    println!("archive_manifest={}", manifest_path.display());
    Ok(())
}

fn run_resume(args: &[String]) -> Result<()> {
    let manifest_path = args
        .first()
        .map(PathBuf::from)
        .context("resume requires a path to archive_manifest.json")?;
    let mut manifest: ArchiveRunManifest = read_json(&manifest_path)?;
    let output_root = PathBuf::from(&manifest.job.output_root);
    let client = DownloadClient::with_cache_dir(None)?;

    execute_archive_job(
        &client,
        &manifest.job.clone(),
        &output_root,
        &mut manifest,
        true,
    )?;
    write_json(&manifest_path, &manifest)?;
    println!("archive_manifest={}", manifest_path.display());
    Ok(())
}

fn run_decode(args: &[String]) -> Result<()> {
    let staged_manifest_path = args
        .first()
        .map(PathBuf::from)
        .context("decode requires a path to a staged subset manifest json")?;
    let staged: StagedSubset = read_json(&staged_manifest_path)?;
    let bundle = decode_field_bundle(&staged.local_grib_path, &staged.local_plan)?;
    let summary = summarize_field_bundle(&bundle);
    let summary_path = staged_manifest_path.with_extension("bundle.json");
    write_json(&summary_path, &summary)?;

    println!(
        "decoded source={} cycle={} f{:02} fields_2d={} fields_3d={} subset_path={}",
        staged.source_plan.source_name,
        staged.source_plan.run.cycle.format("%Y%m%d%H"),
        staged.source_plan.run.forecast_hour,
        summary.fields_2d.len(),
        summary.fields_3d.len(),
        staged.local_grib_path.display()
    );
    println!("bundle_summary={}", summary_path.display());
    Ok(())
}

fn execute_archive_job(
    client: &DownloadClient,
    job: &ArchiveJobSpec,
    output_root: &Path,
    manifest: &mut ArchiveRunManifest,
    decode_after_download: bool,
) -> Result<()> {
    std::fs::create_dir_all(output_root)?;
    let staged_dir = output_root.join("staged");
    let decoded_dir = output_root.join("decoded");
    std::fs::create_dir_all(&staged_dir)?;
    std::fs::create_dir_all(&decoded_dir)?;

    let requests = build_hrrr_archive_requests(job)?;
    for request in requests {
        let record = manifest
            .cycles
            .iter_mut()
            .find(|cycle| cycle.cycle == request.cycle)
            .with_context(|| format!("missing manifest record for cycle {}", request.cycle))?;

        let result = if record.state == ArchiveCycleState::Completed {
            Ok(())
        } else if decode_after_download
            && record.state == ArchiveCycleState::Downloaded
            && record
                .staged_manifest_path
                .as_ref()
                .is_some_and(|path| Path::new(path).exists())
        {
            let staged_manifest_path = PathBuf::from(
                record
                    .staged_manifest_path
                    .as_deref()
                    .context("downloaded cycle was missing a staged manifest path")?,
            );
            let staged: StagedSubset = read_json(&staged_manifest_path)?;
            let bundle = decode_field_bundle(&staged.local_grib_path, &staged.local_plan)?;
            let summary = summarize_field_bundle(&bundle);
            let summary_path = decoded_dir.join(summary_file_name(&staged.source_plan));
            write_json(&summary_path, &summary)?;
            record.decoded_summary_path = Some(summary_path.to_string_lossy().to_string());
            record.field_count_2d = summary.fields_2d.len();
            record.field_count_3d = summary.fields_3d.len();
            record.state = ArchiveCycleState::Completed;
            record.error = None;
            Ok(())
        } else {
            process_cycle(
                client,
                &request,
                &staged_dir,
                &decoded_dir,
                record,
                decode_after_download,
            )
        };

        if let Err(error) = result {
            record.state = ArchiveCycleState::Failed;
            record.error = Some(error.to_string());
        }
        manifest.updated_at = Utc::now();
        write_json(&output_root.join("archive_manifest.json"), manifest)?;
    }

    Ok(())
}

fn process_cycle(
    client: &DownloadClient,
    request: &HrrrSubsetRequest,
    staged_dir: &Path,
    decoded_dir: &Path,
    record: &mut wx_types::ArchiveCycleRecord,
    decode_after_download: bool,
) -> Result<()> {
    let candidate = probe_first_available_hrrr_source(client, request)?;
    let idx_text = fetch_remote_idx_text(client, &candidate)?;
    let plan = plan_hrrr_remote_subset(request, &idx_text, &candidate)?;
    let staged_path = staged_dir.join(stage_subset_file_name(&plan));
    let staged = stage_remote_subset_download(client, &plan, &staged_path)?;
    let staged_manifest_path = staged_dir.join(staged_manifest_file_name(&plan));
    write_json(&staged_manifest_path, &staged)?;

    record.source_name = Some(candidate.name.clone());
    record.source_grib_url = Some(candidate.grib_url.clone());
    record.source_idx_url = Some(candidate.idx_url.clone());
    record.subset_path = Some(staged.local_grib_path.to_string_lossy().to_string());
    record.staged_manifest_path = Some(staged_manifest_path.to_string_lossy().to_string());
    record.message_count = staged.local_plan.selections.len();
    record.state = ArchiveCycleState::Downloaded;
    record.error = None;

    println!(
        "downloaded cycle={} source={} product={} f{:02} messages={} staged_bytes={} subset_path={}",
        request.cycle.format("%Y%m%d%H"),
        candidate.name,
        request.product,
        request.forecast_hour,
        staged.local_plan.selections.len(),
        staged.staged_bytes,
        staged.local_grib_path.display()
    );

    if decode_after_download {
        let bundle = decode_field_bundle(&staged.local_grib_path, &staged.local_plan)?;
        let summary = summarize_field_bundle(&bundle);
        let summary_path = decoded_dir.join(summary_file_name(&plan));
        write_json(&summary_path, &summary)?;
        record.decoded_summary_path = Some(summary_path.to_string_lossy().to_string());
        record.field_count_2d = summary.fields_2d.len();
        record.field_count_3d = summary.fields_3d.len();
        record.state = ArchiveCycleState::Completed;
        println!(
            "decoded cycle={} fields_2d={} fields_3d={} bundle_summary={}",
            request.cycle.format("%Y%m%d%H"),
            summary.fields_2d.len(),
            summary.fields_3d.len(),
            summary_path.display()
        );
    }

    Ok(())
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

fn staged_manifest_file_name(plan: &wx_fetch::SubsetPlan) -> String {
    format!(
        "{}_{}_f{:02}_{}.json",
        plan.source.model,
        plan.run.cycle.format("%Y%m%d%H"),
        plan.run.forecast_hour,
        plan.source.product
    )
}

fn summary_file_name(plan: &wx_fetch::SubsetPlan) -> String {
    format!(
        "{}_{}_f{:02}_{}_bundle.json",
        plan.source.model,
        plan.run.cycle.format("%Y%m%d%H"),
        plan.run.forecast_hour,
        plan.source.product
    )
}

#[derive(Debug, Clone)]
struct ParsedArchiveArgs {
    start_cycle: DateTime<Utc>,
    end_cycle: DateTime<Utc>,
    product: String,
    forecast_hour: u16,
    cycle_step_hours: u16,
    output_root: Option<PathBuf>,
    selections: Vec<RequestedField>,
}

impl ParsedArchiveArgs {
    fn job(&self, output_root: impl Into<String>) -> ArchiveJobSpec {
        ArchiveJobSpec {
            model: "hrrr".to_string(),
            product: self.product.clone(),
            start_cycle: self.start_cycle,
            end_cycle: self.end_cycle,
            cycle_step_hours: self.cycle_step_hours,
            forecast_hour: self.forecast_hour,
            selections: self.selections.clone(),
            output_root: output_root.into(),
        }
    }
}

fn parse_archive_args(args: &[String], expects_output_root: bool) -> Result<ParsedArchiveArgs> {
    let mut cycle_step_hours = 1u16;
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--step" {
            let value = args
                .get(index + 1)
                .context("--step requires an integer hour value")?;
            cycle_step_hours = value
                .parse::<u16>()
                .with_context(|| format!("invalid --step value {value}"))?;
            index += 2;
            continue;
        }
        positionals.push(args[index].clone());
        index += 1;
    }

    let minimum_positionals = if expects_output_root { 6 } else { 5 };
    if positionals.len() < minimum_positionals {
        bail!("not enough arguments for archive command");
    }

    let start_cycle = parse_cycle(&positionals[0])?;
    let end_cycle = parse_cycle(&positionals[1])?;
    let product = positionals[2].clone();
    let forecast_hour = positionals[3]
        .parse::<u16>()
        .with_context(|| format!("invalid forecast hour {}", positionals[3]))?;

    let (output_root, selector_start) = if expects_output_root {
        (Some(PathBuf::from(&positionals[4])), 5)
    } else {
        (None, 4)
    };

    let selections = positionals[selector_start..]
        .iter()
        .map(|selector| parse_selector(selector))
        .collect::<Result<Vec<_>>>()?;
    if selections.is_empty() {
        bail!("at least one selector is required");
    }

    Ok(ParsedArchiveArgs {
        start_cycle,
        end_cycle,
        product,
        forecast_hour,
        cycle_step_hours,
        output_root,
        selections,
    })
}

fn parse_selector(selector: &str) -> Result<RequestedField> {
    let parts: Vec<&str> = selector.split('|').collect();
    match parts.as_slice() {
        [variable, level] => Ok(RequestedField {
            variable: variable.trim().to_string(),
            level: level.trim().to_string(),
            forecast: None,
        }),
        [variable, level, forecast] => Ok(RequestedField {
            variable: variable.trim().to_string(),
            level: level.trim().to_string(),
            forecast: Some(forecast.trim().to_string()),
        }),
        _ => bail!(
            "selector {} is invalid; expected \"VAR|LEVEL\" or \"VAR|LEVEL|FORECAST\"",
            selector
        ),
    }
}

fn parse_cycle(value: &str) -> Result<DateTime<Utc>> {
    if value.len() != 10 {
        bail!("invalid cycle {value}; expected YYYYMMDDHH");
    }
    let naive = chrono::NaiveDateTime::parse_from_str(&format!("{value}00"), "%Y%m%d%H%M")
        .with_context(|| format!("invalid cycle {value}; expected YYYYMMDDHH"))?;
    Ok(Utc.from_utc_datetime(&naive))
}

fn print_usage() {
    println!("usage:");
    println!("  cargo run -p wx-cli -- status");
    println!("  cargo run -p wx-cli -- demo");
    println!(
        "  cargo run -p wx-cli -- plan [--step HOURS] <start_cycle> <end_cycle> <product> <forecast_hour> <selector...>"
    );
    println!(
        "  cargo run -p wx-cli -- download [--step HOURS] <start_cycle> <end_cycle> <product> <forecast_hour> <output_root> <selector...>"
    );
    println!("  cargo run -p wx-cli -- decode <staged_subset_manifest.json>");
    println!(
        "  cargo run -p wx-cli -- archive-run [--step HOURS] <start_cycle> <end_cycle> <product> <forecast_hour> <output_root> <selector...>"
    );
    println!("  cargo run -p wx-cli -- resume <archive_manifest.json>");
    println!(
        "selector syntax: \"VAR|LEVEL\" or \"VAR|LEVEL|FORECAST\"; quote selectors that contain spaces"
    );
}

fn surface_request(cycle: chrono::DateTime<Utc>) -> HrrrSubsetRequest {
    HrrrSubsetRequest {
        cycle,
        forecast_hour: 0,
        product: "sfc".to_string(),
        selections: vec![
            HrrrSelectionRequest {
                variable: "GUST".to_string(),
                level: "surface".to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "PRES".to_string(),
                level: "surface".to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "HGT".to_string(),
                level: "surface".to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "TMP".to_string(),
                level: "2 m above ground".to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "DPT".to_string(),
                level: "2 m above ground".to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "UGRD".to_string(),
                level: "10 m above ground".to_string(),
                forecast: Some("anl".to_string()),
            },
            HrrrSelectionRequest {
                variable: "VGRD".to_string(),
                level: "10 m above ground".to_string(),
                forecast: Some("anl".to_string()),
            },
        ],
    }
}

fn pressure_request(cycle: chrono::DateTime<Utc>) -> HrrrSubsetRequest {
    HrrrSubsetRequest {
        cycle,
        forecast_hour: 0,
        product: "prs".to_string(),
        selections: DEMO_PRESSURE_LEVELS
            .into_iter()
            .flat_map(|level| {
                [
                    HrrrSelectionRequest {
                        variable: "HGT".to_string(),
                        level: level.to_string(),
                        forecast: Some("anl".to_string()),
                    },
                    HrrrSelectionRequest {
                        variable: "TMP".to_string(),
                        level: level.to_string(),
                        forecast: Some("anl".to_string()),
                    },
                    HrrrSelectionRequest {
                        variable: "DPT".to_string(),
                        level: level.to_string(),
                        forecast: Some("anl".to_string()),
                    },
                    HrrrSelectionRequest {
                        variable: "UGRD".to_string(),
                        level: level.to_string(),
                        forecast: Some("anl".to_string()),
                    },
                    HrrrSelectionRequest {
                        variable: "VGRD".to_string(),
                        level: level.to_string(),
                        forecast: Some("anl".to_string()),
                    },
                ]
            })
            .collect(),
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace layout is stable")
        .to_path_buf()
}
