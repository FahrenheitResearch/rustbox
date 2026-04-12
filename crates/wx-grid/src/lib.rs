use anyhow::{Result, bail};
use rayon::prelude::*;
use wx_types::{Field2D, FieldMetadata};

#[derive(Debug, Clone, PartialEq)]
pub struct FieldSummary {
    pub finite_count: usize,
    pub min_value: f32,
    pub max_value: f32,
    pub mean_value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSpec {
    pub window_nx: usize,
    pub window_ny: usize,
    pub passes: usize,
    pub normalize_weights: bool,
}

const RD_DRY_AIR: f64 = 287.05;
const CP_DRY_AIR: f64 = 1004.0;
const KAPPA: f64 = RD_DRY_AIR / CP_DRY_AIR;

pub fn summarize_grid(field: &Field2D) -> Option<(f32, f32)> {
    field_stats(field).map(|summary| (summary.min_value, summary.max_value))
}

pub fn field_stats(field: &Field2D) -> Option<FieldSummary> {
    let mut finite_count = 0usize;
    let mut min_value = f32::INFINITY;
    let mut max_value = f32::NEG_INFINITY;
    let mut sum = 0.0f64;

    for value in &field.values {
        if value.is_finite() {
            finite_count += 1;
            min_value = min_value.min(*value);
            max_value = max_value.max(*value);
            sum += *value as f64;
        }
    }

    if finite_count == 0 {
        None
    } else {
        Some(FieldSummary {
            finite_count,
            min_value,
            max_value,
            mean_value: sum / finite_count as f64,
        })
    }
}

pub fn gradient_x(values: &[f64], nx: usize, ny: usize, dx: f64) -> Vec<f64> {
    assert_eq!(values.len(), nx * ny, "values length must equal nx * ny");
    let inv_2dx = 1.0 / (2.0 * dx);
    let inv_dx = 1.0 / dx;

    let rows: Vec<Vec<f64>> = (0..ny)
        .into_par_iter()
        .map(|j| {
            (0..nx)
                .map(|i| {
                    if nx < 2 {
                        0.0
                    } else if nx == 2 {
                        (values[idx(j, 1, nx)] - values[idx(j, 0, nx)]) * inv_dx
                    } else if i == 0 {
                        (-3.0 * values[idx(j, 0, nx)] + 4.0 * values[idx(j, 1, nx)]
                            - values[idx(j, 2, nx)])
                            * inv_2dx
                    } else if i == nx - 1 {
                        (3.0 * values[idx(j, nx - 1, nx)] - 4.0 * values[idx(j, nx - 2, nx)]
                            + values[idx(j, nx - 3, nx)])
                            * inv_2dx
                    } else {
                        (values[idx(j, i + 1, nx)] - values[idx(j, i - 1, nx)]) * inv_2dx
                    }
                })
                .collect()
        })
        .collect();
    rows.into_iter().flatten().collect()
}

pub fn gradient_y(values: &[f64], nx: usize, ny: usize, dy: f64) -> Vec<f64> {
    assert_eq!(values.len(), nx * ny, "values length must equal nx * ny");
    let inv_2dy = 1.0 / (2.0 * dy);
    let inv_dy = 1.0 / dy;

    let rows: Vec<Vec<f64>> = (0..ny)
        .into_par_iter()
        .map(|j| {
            (0..nx)
                .map(|i| {
                    if ny < 2 {
                        0.0
                    } else if ny == 2 {
                        (values[idx(1, i, nx)] - values[idx(0, i, nx)]) * inv_dy
                    } else if j == 0 {
                        (-3.0 * values[idx(0, i, nx)] + 4.0 * values[idx(1, i, nx)]
                            - values[idx(2, i, nx)])
                            * inv_2dy
                    } else if j == ny - 1 {
                        (3.0 * values[idx(ny - 1, i, nx)] - 4.0 * values[idx(ny - 2, i, nx)]
                            + values[idx(ny - 3, i, nx)])
                            * inv_2dy
                    } else {
                        (values[idx(j + 1, i, nx)] - values[idx(j - 1, i, nx)]) * inv_2dy
                    }
                })
                .collect()
        })
        .collect();
    rows.into_iter().flatten().collect()
}

pub fn laplacian(values: &[f64], nx: usize, ny: usize, dx: f64, dy: f64) -> Vec<f64> {
    assert_eq!(values.len(), nx * ny, "values length must equal nx * ny");
    let inv_dx2 = 1.0 / (dx * dx);
    let inv_dy2 = 1.0 / (dy * dy);

    let rows: Vec<Vec<f64>> = (0..ny)
        .into_par_iter()
        .map(|j| {
            (0..nx)
                .map(|i| {
                    let d2x = if nx < 3 {
                        0.0
                    } else if i == 0 {
                        (values[idx(j, 2, nx)] - 2.0 * values[idx(j, 1, nx)]
                            + values[idx(j, 0, nx)])
                            * inv_dx2
                    } else if i == nx - 1 {
                        (values[idx(j, nx - 1, nx)] - 2.0 * values[idx(j, nx - 2, nx)]
                            + values[idx(j, nx - 3, nx)])
                            * inv_dx2
                    } else {
                        (values[idx(j, i + 1, nx)] - 2.0 * values[idx(j, i, nx)]
                            + values[idx(j, i - 1, nx)])
                            * inv_dx2
                    };
                    let d2y = if ny < 3 {
                        0.0
                    } else if j == 0 {
                        (values[idx(2, i, nx)] - 2.0 * values[idx(1, i, nx)]
                            + values[idx(0, i, nx)])
                            * inv_dy2
                    } else if j == ny - 1 {
                        (values[idx(ny - 1, i, nx)] - 2.0 * values[idx(ny - 2, i, nx)]
                            + values[idx(ny - 3, i, nx)])
                            * inv_dy2
                    } else {
                        (values[idx(j + 1, i, nx)] - 2.0 * values[idx(j, i, nx)]
                            + values[idx(j - 1, i, nx)])
                            * inv_dy2
                    };
                    d2x + d2y
                })
                .collect()
        })
        .collect();
    rows.into_iter().flatten().collect()
}

pub fn divergence(u: &[f64], v: &[f64], nx: usize, ny: usize, dx: f64, dy: f64) -> Vec<f64> {
    let dudx = gradient_x(u, nx, ny, dx);
    let dvdy = gradient_y(v, nx, ny, dy);
    dudx.par_iter()
        .zip(dvdy.par_iter())
        .map(|(a, b)| a + b)
        .collect()
}

pub fn vorticity(u: &[f64], v: &[f64], nx: usize, ny: usize, dx: f64, dy: f64) -> Vec<f64> {
    let dvdx = gradient_x(v, nx, ny, dx);
    let dudy = gradient_y(u, nx, ny, dy);
    dvdx.par_iter()
        .zip(dudy.par_iter())
        .map(|(a, b)| a - b)
        .collect()
}

pub fn advection(
    scalar: &[f64],
    u: &[f64],
    v: &[f64],
    nx: usize,
    ny: usize,
    dx: f64,
    dy: f64,
) -> Vec<f64> {
    let dsdx = gradient_x(scalar, nx, ny, dx);
    let dsdy = gradient_y(scalar, nx, ny, dy);
    (0..nx * ny)
        .into_par_iter()
        .map(|index| -u[index] * dsdx[index] - v[index] * dsdy[index])
        .collect()
}

pub fn frontogenesis(
    theta: &[f64],
    u: &[f64],
    v: &[f64],
    nx: usize,
    ny: usize,
    dx: f64,
    dy: f64,
) -> Vec<f64> {
    let dtdx = gradient_x(theta, nx, ny, dx);
    let dtdy = gradient_y(theta, nx, ny, dy);
    let dudx = gradient_x(u, nx, ny, dx);
    let dvdy = gradient_y(v, nx, ny, dy);
    let dvdx = gradient_x(v, nx, ny, dx);
    let dudy = gradient_y(u, nx, ny, dy);

    let mut out = vec![0.0; nx * ny];
    for index in 0..out.len() {
        let gradient_magnitude = (dtdx[index] * dtdx[index] + dtdy[index] * dtdy[index]).sqrt();
        if gradient_magnitude < 1e-20 {
            continue;
        }

        out[index] = -(dtdx[index] * dtdx[index] * dudx[index]
            + dtdy[index] * dtdy[index] * dvdy[index]
            + dtdx[index] * dtdy[index] * (dvdx[index] + dudy[index]))
            / gradient_magnitude;
    }

    out
}

pub fn wind_speed(u: &[f64], v: &[f64]) -> Vec<f64> {
    assert_eq!(u.len(), v.len(), "u and v must have the same length");
    u.par_iter()
        .zip(v.par_iter())
        .map(|(u_value, v_value)| u_value.hypot(*v_value))
        .collect()
}

pub fn temperature_kelvin_to_celsius(temperature_k: &[f64]) -> Vec<f64> {
    temperature_k
        .par_iter()
        .map(|value| value - 273.15)
        .collect()
}

pub fn relative_humidity_from_dewpoint(temperature_c: &[f64], dewpoint_c: &[f64]) -> Vec<f64> {
    assert_eq!(
        temperature_c.len(),
        dewpoint_c.len(),
        "temperature and dewpoint must have the same length"
    );
    temperature_c
        .par_iter()
        .zip(dewpoint_c.par_iter())
        .map(|(temperature_c, dewpoint_c)| {
            let es = 6.112 * ((17.67 * temperature_c) / (temperature_c + 243.5)).exp();
            let e = 6.112 * ((17.67 * dewpoint_c) / (dewpoint_c + 243.5)).exp();
            if es.is_finite() && es > 0.0 {
                (100.0 * e / es).clamp(0.0, 100.0)
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn potential_temperature(temperature_k: &[f64], pressure_hpa: f64) -> Vec<f64> {
    let theta_factor = (1000.0 / pressure_hpa).powf(KAPPA);
    temperature_k
        .par_iter()
        .map(|temperature| temperature * theta_factor)
        .collect()
}

pub fn thickness(height_lower_m: &[f64], height_upper_m: &[f64]) -> Vec<f64> {
    assert_eq!(
        height_lower_m.len(),
        height_upper_m.len(),
        "lower and upper height fields must have the same length"
    );
    height_lower_m
        .par_iter()
        .zip(height_upper_m.par_iter())
        .map(|(lower, upper)| upper - lower)
        .collect()
}

pub fn lapse_rate(
    temperature_lower_k: &[f64],
    temperature_upper_k: &[f64],
    height_lower_m: &[f64],
    height_upper_m: &[f64],
) -> Vec<f64> {
    assert_eq!(
        temperature_lower_k.len(),
        temperature_upper_k.len(),
        "lower and upper temperature fields must have the same length"
    );
    assert_eq!(
        height_lower_m.len(),
        height_upper_m.len(),
        "lower and upper height fields must have the same length"
    );
    assert_eq!(
        temperature_lower_k.len(),
        height_lower_m.len(),
        "temperature and height fields must have the same length"
    );
    temperature_lower_k
        .par_iter()
        .zip(temperature_upper_k.par_iter())
        .zip(height_lower_m.par_iter().zip(height_upper_m.par_iter()))
        .map(|((temp_lower, temp_upper), (height_lower, height_upper))| {
            let dz_m = height_upper - height_lower;
            if dz_m.abs() < 1.0 {
                f64::NAN
            } else {
                (temp_lower - temp_upper) / (dz_m / 1_000.0)
            }
        })
        .collect()
}

pub fn smooth_window(
    data: &[f64],
    nx: usize,
    ny: usize,
    window: &[f64],
    spec: WindowSpec,
) -> Vec<f64> {
    let expected_len = nx * ny;
    assert_eq!(data.len(), expected_len, "data length must equal nx * ny");
    assert_eq!(
        window.len(),
        spec.window_nx * spec.window_ny,
        "window length must equal window_nx * window_ny"
    );
    assert!(spec.window_nx > 0, "window_nx must be > 0");
    assert!(spec.window_ny > 0, "window_ny must be > 0");

    let half_x = spec.window_nx / 2;
    let half_y = spec.window_ny / 2;
    let weights: Vec<f64> = if spec.normalize_weights {
        let weight_sum: f64 = window.iter().sum();
        if weight_sum.abs() > 1e-30 {
            window.iter().map(|weight| weight / weight_sum).collect()
        } else {
            window.to_vec()
        }
    } else {
        window.to_vec()
    };

    let mut current = data.to_vec();
    for _ in 0..spec.passes {
        let mut out = current.clone();
        let j_start = half_y;
        let j_end = ny.saturating_sub(half_y);

        if j_end > j_start {
            let interior_rows = j_end - j_start;
            let mut interior = vec![0.0; interior_rows * nx];

            interior
                .par_chunks_mut(nx)
                .enumerate()
                .for_each(|(row_index, row)| {
                    let j = j_start + row_index;
                    row.copy_from_slice(&current[j * nx..(j + 1) * nx]);

                    for (i, cell) in row
                        .iter_mut()
                        .enumerate()
                        .take(nx.saturating_sub(half_x))
                        .skip(half_x)
                    {
                        let mut weighted_sum = 0.0;
                        let mut has_nan = false;

                        'kernel: for window_j in 0..spec.window_ny {
                            let jj = j + window_j - half_y;
                            for window_i in 0..spec.window_nx {
                                let ii = i + window_i - half_x;
                                let value = current[jj * nx + ii];
                                if value.is_nan() {
                                    has_nan = true;
                                    break 'kernel;
                                }
                                weighted_sum +=
                                    weights[window_j * spec.window_nx + window_i] * value;
                            }
                        }

                        *cell = if has_nan { f64::NAN } else { weighted_sum };
                    }
                });

            for (row_index, chunk) in interior.chunks(nx).enumerate() {
                let j = j_start + row_index;
                out[j * nx..(j + 1) * nx].copy_from_slice(chunk);
            }
        }

        current = out;
    }

    current
}

pub fn smooth_n_point(data: &[f64], nx: usize, ny: usize, n: usize, passes: usize) -> Vec<f64> {
    assert!(n == 5 || n == 9, "n must be 5 or 9, got {n}");
    let kernel = if n == 9 {
        vec![
            0.0625, 0.125, 0.0625, 0.125, 0.25, 0.125, 0.0625, 0.125, 0.0625,
        ]
    } else {
        vec![0.0, 0.125, 0.0, 0.125, 0.5, 0.125, 0.0, 0.125, 0.0]
    };

    smooth_window(
        data,
        nx,
        ny,
        &kernel,
        WindowSpec {
            window_nx: 3,
            window_ny: 3,
            passes,
            normalize_weights: false,
        },
    )
}

fn wind_speed_from_components(u: &[f64], v: &[f64]) -> Vec<f64> {
    wind_speed(u, v)
}

fn wind_direction_from_components(u: &[f64], v: &[f64]) -> Vec<f64> {
    assert_eq!(u.len(), v.len(), "u and v must have the same length");
    u.par_iter()
        .zip(v.par_iter())
        .map(|(u_value, v_value)| {
            if u_value.abs() < 1e-12 && v_value.abs() < 1e-12 {
                0.0
            } else {
                (-*u_value).atan2(-*v_value).to_degrees().rem_euclid(360.0)
            }
        })
        .collect()
}

pub fn gradient_x_field(field: &Field2D) -> Result<Field2D> {
    validate_scalar_field(field)?;
    let units = format!("{}/m", field.metadata.units);
    derived_scalar_field(
        field,
        "DTDX",
        "X gradient",
        &units,
        gradient_x(
            &field_to_f64(field),
            field.grid.nx,
            field.grid.ny,
            field.grid.coordinates.dx,
        ),
    )
}

pub fn gradient_y_field(field: &Field2D) -> Result<Field2D> {
    validate_scalar_field(field)?;
    let units = format!("{}/m", field.metadata.units);
    derived_scalar_field(
        field,
        "DTDY",
        "Y gradient",
        &units,
        gradient_y(
            &field_to_f64(field),
            field.grid.nx,
            field.grid.ny,
            field.grid.coordinates.dy,
        ),
    )
}

pub fn laplacian_field(field: &Field2D) -> Result<Field2D> {
    validate_scalar_field(field)?;
    let units = format!("{}/m^2", field.metadata.units);
    derived_scalar_field(
        field,
        "LAPL",
        "Laplacian",
        &units,
        laplacian(
            &field_to_f64(field),
            field.grid.nx,
            field.grid.ny,
            field.grid.coordinates.dx,
            field.grid.coordinates.dy,
        ),
    )
}

pub fn divergence_field(u_field: &Field2D, v_field: &Field2D) -> Result<Field2D> {
    validate_wind_component_pair(u_field, v_field)?;
    derived_scalar_field(
        u_field,
        "DIV",
        "Horizontal divergence",
        "s^-1",
        divergence(
            &field_to_f64(u_field),
            &field_to_f64(v_field),
            u_field.grid.nx,
            u_field.grid.ny,
            u_field.grid.coordinates.dx,
            u_field.grid.coordinates.dy,
        ),
    )
}

pub fn wind_speed_field(u_field: &Field2D, v_field: &Field2D) -> Result<Field2D> {
    validate_wind_component_pair(u_field, v_field)?;
    derived_scalar_field(
        u_field,
        "WSPD",
        "Wind Speed",
        "m/s",
        wind_speed_from_components(&field_to_f64(u_field), &field_to_f64(v_field)),
    )
}

pub fn wind_direction_field(u_field: &Field2D, v_field: &Field2D) -> Result<Field2D> {
    validate_wind_component_pair(u_field, v_field)?;
    derived_scalar_field(
        u_field,
        "WDIR",
        "Wind Direction",
        "degrees",
        wind_direction_from_components(&field_to_f64(u_field), &field_to_f64(v_field)),
    )
}

pub fn vorticity_field(u_field: &Field2D, v_field: &Field2D) -> Result<Field2D> {
    validate_wind_component_pair(u_field, v_field)?;
    derived_scalar_field(
        u_field,
        "VORT",
        "Relative vorticity",
        "s^-1",
        vorticity(
            &field_to_f64(u_field),
            &field_to_f64(v_field),
            u_field.grid.nx,
            u_field.grid.ny,
            u_field.grid.coordinates.dx,
            u_field.grid.coordinates.dy,
        ),
    )
}

pub fn advection_field(
    scalar_field: &Field2D,
    u_field: &Field2D,
    v_field: &Field2D,
) -> Result<Field2D> {
    validate_scalar_field(scalar_field)?;
    validate_vector_pair(u_field, v_field)?;
    ensure_compatible_fields(scalar_field, u_field, "scalar and u-component")?;
    ensure_compatible_fields(scalar_field, v_field, "scalar and v-component")?;
    let units = format!("{}/s", scalar_field.metadata.units);

    derived_scalar_field(
        scalar_field,
        "ADV",
        "Scalar advection",
        &units,
        advection(
            &field_to_f64(scalar_field),
            &field_to_f64(u_field),
            &field_to_f64(v_field),
            scalar_field.grid.nx,
            scalar_field.grid.ny,
            scalar_field.grid.coordinates.dx,
            scalar_field.grid.coordinates.dy,
        ),
    )
}

pub fn temperature_celsius_field(temperature_field: &Field2D) -> Result<Field2D> {
    validate_temperature_field(temperature_field)?;
    derived_scalar_field(
        temperature_field,
        "TMP_C",
        "Temperature",
        "C",
        temperature_kelvin_to_celsius(&field_to_f64(temperature_field)),
    )
}

pub fn dewpoint_celsius_field(dewpoint_field: &Field2D) -> Result<Field2D> {
    validate_dewpoint_field(dewpoint_field)?;
    derived_scalar_field(
        dewpoint_field,
        "DPT_C",
        "Dewpoint Temperature",
        "C",
        temperature_kelvin_to_celsius(&field_to_f64(dewpoint_field)),
    )
}

pub fn relative_humidity_field(
    temperature_field: &Field2D,
    dewpoint_field: &Field2D,
) -> Result<Field2D> {
    validate_temperature_field(temperature_field)?;
    validate_dewpoint_field(dewpoint_field)?;
    ensure_compatible_fields(
        temperature_field,
        dewpoint_field,
        "temperature and dewpoint",
    )?;
    let temperature_c = temperature_kelvin_to_celsius(&field_to_f64(temperature_field));
    let dewpoint_c = temperature_kelvin_to_celsius(&field_to_f64(dewpoint_field));
    derived_scalar_field(
        temperature_field,
        "RH",
        "Relative Humidity",
        "%",
        relative_humidity_from_dewpoint(&temperature_c, &dewpoint_c),
    )
}

pub fn frontogenesis_field(
    theta_field: &Field2D,
    u_field: &Field2D,
    v_field: &Field2D,
) -> Result<Field2D> {
    validate_theta_field(theta_field)?;
    validate_wind_component_pair(u_field, v_field)?;
    ensure_compatible_fields(theta_field, u_field, "theta and u-component")?;
    ensure_compatible_fields(theta_field, v_field, "theta and v-component")?;

    derived_scalar_field(
        theta_field,
        "FGEN",
        "Petterssen frontogenesis",
        "K m^-1 s^-1",
        frontogenesis(
            &field_to_f64(theta_field),
            &field_to_f64(u_field),
            &field_to_f64(v_field),
            theta_field.grid.nx,
            theta_field.grid.ny,
            theta_field.grid.coordinates.dx,
            theta_field.grid.coordinates.dy,
        ),
    )
}

pub fn potential_temperature_field(temperature_field: &Field2D) -> Result<Field2D> {
    validate_temperature_field(temperature_field)?;
    let pressure_hpa = isobaric_level_hpa(temperature_field)?;
    derived_scalar_field(
        temperature_field,
        "THETA",
        "Potential Temperature",
        "K",
        potential_temperature(&field_to_f64(temperature_field), pressure_hpa),
    )
}

pub fn pressure_level_frontogenesis_field(
    temperature_field: &Field2D,
    u_field: &Field2D,
    v_field: &Field2D,
) -> Result<Field2D> {
    validate_temperature_field(temperature_field)?;
    validate_wind_component_pair(u_field, v_field)?;
    ensure_compatible_fields(temperature_field, u_field, "temperature and u-component")?;
    ensure_compatible_fields(temperature_field, v_field, "temperature and v-component")?;

    let theta_field = potential_temperature_field(temperature_field)?;
    frontogenesis_field(&theta_field, u_field, v_field)
}

pub fn smooth_n_point_field(field: &Field2D, n: usize, passes: usize) -> Result<Field2D> {
    validate_scalar_field(field)?;
    derived_scalar_field(
        field,
        if n == 5 { "SM5" } else { "SM9" },
        &format!("{n}-point smoothed {}", field.metadata.parameter),
        &field.metadata.units,
        smooth_n_point(
            &field_to_f64(field),
            field.grid.nx,
            field.grid.ny,
            n,
            passes,
        ),
    )
}

pub fn thickness_field(
    lower_height_field: &Field2D,
    upper_height_field: &Field2D,
) -> Result<Field2D> {
    validate_height_field(lower_height_field)?;
    validate_height_field(upper_height_field)?;
    ensure_cross_level_compatible_fields(
        lower_height_field,
        upper_height_field,
        "lower and upper height",
    )?;
    if lower_height_field.metadata.units != upper_height_field.metadata.units {
        bail!(
            "height fields use incompatible units {} and {}",
            lower_height_field.metadata.units,
            upper_height_field.metadata.units
        );
    }

    let lower_pressure = isobaric_level_hpa(lower_height_field)?;
    let upper_pressure = isobaric_level_hpa(upper_height_field)?;
    if lower_pressure <= upper_pressure {
        bail!(
            "thickness requires lower level pressure > upper level pressure, got {} and {} hPa",
            lower_pressure,
            upper_pressure
        );
    }

    derived_scalar_field(
        lower_height_field,
        "THICK",
        "Layer thickness",
        &lower_height_field.metadata.units,
        thickness(
            &field_to_f64(lower_height_field),
            &field_to_f64(upper_height_field),
        ),
    )
}

pub fn lapse_rate_field(
    lower_temperature_field: &Field2D,
    upper_temperature_field: &Field2D,
    lower_height_field: &Field2D,
    upper_height_field: &Field2D,
) -> Result<Field2D> {
    validate_temperature_field(lower_temperature_field)?;
    validate_temperature_field(upper_temperature_field)?;
    validate_height_field(lower_height_field)?;
    validate_height_field(upper_height_field)?;
    ensure_cross_level_compatible_fields(
        lower_temperature_field,
        upper_temperature_field,
        "lower and upper temperature",
    )?;
    ensure_cross_level_compatible_fields(
        lower_height_field,
        upper_height_field,
        "lower and upper height",
    )?;
    ensure_cross_level_source_alignment(
        lower_temperature_field,
        lower_height_field,
        "lower temperature and lower height",
    )?;
    ensure_cross_level_source_alignment(
        upper_temperature_field,
        upper_height_field,
        "upper temperature and upper height",
    )?;

    let lower_pressure = isobaric_level_hpa(lower_temperature_field)?;
    let upper_pressure = isobaric_level_hpa(upper_temperature_field)?;
    if lower_pressure <= upper_pressure {
        bail!(
            "lapse rate requires lower level pressure > upper level pressure, got {} and {} hPa",
            lower_pressure,
            upper_pressure
        );
    }

    derived_scalar_field(
        lower_temperature_field,
        "LRATE",
        "Lapse Rate",
        "K/km",
        lapse_rate(
            &field_to_f64(lower_temperature_field),
            &field_to_f64(upper_temperature_field),
            &field_to_f64(lower_height_field),
            &field_to_f64(upper_height_field),
        ),
    )
}

pub fn anomaly_field(field: &Field2D) -> Result<Field2D> {
    validate_scalar_field(field)?;
    let stats = field_stats(field).ok_or_else(|| {
        anyhow::anyhow!(
            "field {} contained no finite values for anomaly calculation",
            field.metadata.short_name
        )
    })?;
    let mean = stats.mean_value;
    derived_scalar_field(
        field,
        "ANOM",
        &format!("{} anomaly", field.metadata.parameter),
        &field.metadata.units,
        field_to_f64(field)
            .into_iter()
            .map(|value| {
                if value.is_finite() {
                    value - mean
                } else {
                    f64::NAN
                }
            })
            .collect(),
    )
}

fn validate_scalar_field(field: &Field2D) -> Result<()> {
    if field.values.len() != field.expected_len() {
        bail!(
            "field {} has {} values but grid expects {}",
            field.metadata.short_name,
            field.values.len(),
            field.expected_len()
        );
    }
    if field.grid.coordinates.dx <= 0.0 || field.grid.coordinates.dy <= 0.0 {
        bail!(
            "field {} has non-positive grid spacing dx={} dy={}",
            field.metadata.short_name,
            field.grid.coordinates.dx,
            field.grid.coordinates.dy
        );
    }
    Ok(())
}

fn validate_vector_pair(u_field: &Field2D, v_field: &Field2D) -> Result<()> {
    validate_scalar_field(u_field)?;
    validate_scalar_field(v_field)?;
    ensure_compatible_fields(u_field, v_field, "u and v wind components")
}

fn validate_wind_component_pair(u_field: &Field2D, v_field: &Field2D) -> Result<()> {
    validate_vector_pair(u_field, v_field)?;
    validate_wind_component(u_field, "UGRD", "U-Component of Wind")?;
    validate_wind_component(v_field, "VGRD", "V-Component of Wind")?;
    Ok(())
}

fn validate_wind_component(field: &Field2D, short_name: &str, parameter: &str) -> Result<()> {
    if field.metadata.short_name != short_name || field.metadata.parameter != parameter {
        bail!(
            "field {} / {} is not the expected {} wind component",
            field.metadata.short_name,
            field.metadata.parameter,
            short_name
        );
    }
    if field.metadata.units != "m/s" {
        bail!(
            "field {} uses units {}, expected m/s",
            field.metadata.short_name,
            field.metadata.units
        );
    }
    Ok(())
}

fn validate_temperature_field(field: &Field2D) -> Result<()> {
    validate_scalar_field(field)?;
    if field.metadata.short_name != "TMP" || field.metadata.parameter != "Temperature" {
        bail!(
            "field {} / {} is not a temperature field",
            field.metadata.short_name,
            field.metadata.parameter
        );
    }
    if field.metadata.units != "K" {
        bail!(
            "temperature field {} uses units {}, expected K",
            field.metadata.short_name,
            field.metadata.units
        );
    }
    Ok(())
}

fn validate_dewpoint_field(field: &Field2D) -> Result<()> {
    validate_scalar_field(field)?;
    let valid_parameter = matches!(
        field.metadata.parameter.as_str(),
        "Dew Point Temperature" | "Dewpoint Temperature"
    );
    if field.metadata.short_name != "DPT" || !valid_parameter {
        bail!(
            "field {} / {} is not a dewpoint field",
            field.metadata.short_name,
            field.metadata.parameter
        );
    }
    if field.metadata.units != "K" {
        bail!(
            "dewpoint field {} uses units {}, expected K",
            field.metadata.short_name,
            field.metadata.units
        );
    }
    Ok(())
}

fn validate_height_field(field: &Field2D) -> Result<()> {
    validate_scalar_field(field)?;
    if field.metadata.short_name != "HGT" || field.metadata.parameter != "Geopotential Height" {
        bail!(
            "field {} / {} is not a geopotential-height field",
            field.metadata.short_name,
            field.metadata.parameter
        );
    }
    if field.metadata.units != "gpm" && field.metadata.units != "m" {
        bail!(
            "height field {} uses units {}, expected gpm or m",
            field.metadata.short_name,
            field.metadata.units
        );
    }
    Ok(())
}

fn validate_theta_field(field: &Field2D) -> Result<()> {
    validate_scalar_field(field)?;
    if field.metadata.short_name != "THETA" || field.metadata.parameter != "Potential Temperature" {
        bail!(
            "field {} / {} is not a potential-temperature field",
            field.metadata.short_name,
            field.metadata.parameter
        );
    }
    if field.metadata.units != "K" {
        bail!(
            "potential-temperature field {} uses units {}, expected K",
            field.metadata.short_name,
            field.metadata.units
        );
    }
    isobaric_level_hpa(field)?;
    Ok(())
}

fn isobaric_level_hpa(field: &Field2D) -> Result<f64> {
    if field.metadata.level.code != 100 {
        bail!(
            "field {} is on level code {}, expected isobaric level code 100",
            field.metadata.short_name,
            field.metadata.level.code
        );
    }
    let pressure_hpa = field.metadata.level.value.ok_or_else(|| {
        anyhow::anyhow!(
            "field {} is missing isobaric level value metadata",
            field.metadata.short_name
        )
    })?;
    if pressure_hpa <= 0.0 {
        bail!(
            "field {} has non-positive pressure level {} hPa",
            field.metadata.short_name,
            pressure_hpa
        );
    }
    if field.metadata.level.units != "hPa" {
        bail!(
            "field {} level units are {}, expected hPa",
            field.metadata.short_name,
            field.metadata.level.units
        );
    }
    Ok(pressure_hpa)
}

fn ensure_compatible_fields(left: &Field2D, right: &Field2D, label: &str) -> Result<()> {
    if left.grid != right.grid {
        bail!("{label} use incompatible grid geometry");
    }
    if left.metadata.source != right.metadata.source {
        bail!("{label} use incompatible source metadata");
    }
    if left.metadata.run != right.metadata.run {
        bail!("{label} use incompatible run metadata");
    }
    if left.metadata.valid != right.metadata.valid {
        bail!("{label} use incompatible valid times");
    }
    if left.metadata.level != right.metadata.level {
        bail!("{label} use incompatible levels");
    }
    Ok(())
}

fn ensure_cross_level_source_alignment(left: &Field2D, right: &Field2D, label: &str) -> Result<()> {
    if left.grid != right.grid {
        bail!("{label} use incompatible grid geometry");
    }
    if left.metadata.source != right.metadata.source {
        bail!("{label} use incompatible source metadata");
    }
    if left.metadata.run != right.metadata.run {
        bail!("{label} use incompatible run metadata");
    }
    if left.metadata.valid != right.metadata.valid {
        bail!("{label} use incompatible valid times");
    }
    Ok(())
}

fn ensure_cross_level_compatible_fields(
    left: &Field2D,
    right: &Field2D,
    label: &str,
) -> Result<()> {
    ensure_cross_level_source_alignment(left, right, label)?;
    if left.metadata.level.code != right.metadata.level.code {
        bail!("{label} use incompatible vertical coordinate types");
    }
    if left.metadata.level.units != right.metadata.level.units {
        bail!("{label} use incompatible vertical coordinate units");
    }
    Ok(())
}

fn derived_scalar_field(
    template: &Field2D,
    short_name: &str,
    parameter: &str,
    units: &str,
    values: Vec<f64>,
) -> Result<Field2D> {
    if values.len() != template.expected_len() {
        bail!(
            "derived field {} produced {} values but grid expects {}",
            short_name,
            values.len(),
            template.expected_len()
        );
    }

    let mut metadata: FieldMetadata = template.metadata.clone();
    metadata.short_name = short_name.to_string();
    metadata.parameter = parameter.to_string();
    metadata.units = units.to_string();

    Ok(Field2D {
        metadata,
        grid: template.grid.clone(),
        values: values.into_iter().map(|value| value as f32).collect(),
    })
}

fn field_to_f64(field: &Field2D) -> Vec<f64> {
    field.values.iter().map(|value| *value as f64).collect()
}

#[inline(always)]
fn idx(j: usize, i: usize, nx: usize) -> usize {
    j * nx + i
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::path::PathBuf;
    use wx_fetch::{HrrrSelectionRequest, HrrrSubsetRequest, plan_hrrr_fixture_subset};
    use wx_grib::decode_selected_messages;

    fn approx(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() < tolerance,
            "expected {expected}, got {actual} (diff {}, tol {tolerance})",
            (actual - expected).abs()
        );
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    fn pressure_fixture_request(cycle: chrono::DateTime<Utc>) -> HrrrSubsetRequest {
        HrrrSubsetRequest {
            cycle,
            forecast_hour: 0,
            product: "prs".to_string(),
            selections: ["1000 mb", "850 mb", "500 mb"]
                .into_iter()
                .flat_map(|level| {
                    ["HGT", "TMP", "DPT", "UGRD", "VGRD"]
                        .into_iter()
                        .map(move |variable| HrrrSelectionRequest {
                            variable: variable.to_string(),
                            level: level.to_string(),
                            forecast: Some("anl".to_string()),
                        })
                })
                .collect(),
        }
    }

    fn decode_pressure_fixture_fields() -> Vec<Field2D> {
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid fixture cycle");
        let fragment = fixture_path("hrrr_demo_pressure_fragment.grib2");
        let idx_text = std::fs::read_to_string(fixture_path("hrrr_demo_pressure_fragment.idx"))
            .expect("pressure idx fixture should be readable");
        let plan = plan_hrrr_fixture_subset(
            &pressure_fixture_request(cycle),
            &idx_text,
            std::fs::metadata(&fragment)
                .expect("pressure fixture should exist")
                .len(),
        )
        .expect("fixture plan should succeed");
        decode_selected_messages(&fragment, &plan)
            .expect("fixture decode should succeed")
            .into_iter()
            .map(|message| message.field)
            .collect()
    }

    fn select_field<'a>(fields: &'a [Field2D], short_name: &str) -> &'a Field2D {
        fields
            .iter()
            .find(|field| field.metadata.short_name == short_name)
            .unwrap_or_else(|| panic!("missing {short_name} in decoded fixture"))
    }

    fn select_field_at_level<'a>(
        fields: &'a [Field2D],
        short_name: &str,
        level_description: &str,
    ) -> &'a Field2D {
        fields
            .iter()
            .find(|field| {
                field.metadata.short_name == short_name
                    && field.metadata.level.description == level_description
            })
            .unwrap_or_else(|| {
                panic!("missing {short_name} at {level_description} in decoded fixture")
            })
    }

    #[test]
    fn linear_field_gradients_are_exact() {
        let nx = 5;
        let ny = 5;
        let dx = 2.0;
        let dy = 4.0;
        let mut values = vec![0.0; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                values[idx(j, i, nx)] = 3.0 * i as f64 + 5.0 * j as f64;
            }
        }

        let dfdx = gradient_x(&values, nx, ny, dx);
        let dfdy = gradient_y(&values, nx, ny, dy);
        for value in dfdx {
            approx(value, 1.5, 1e-10);
        }
        for value in dfdy {
            approx(value, 1.25, 1e-10);
        }
    }

    #[test]
    fn solid_body_rotation_has_zero_divergence_and_two_unit_vorticity() {
        let nx = 7;
        let ny = 7;
        let mut u = vec![0.0; nx * ny];
        let mut v = vec![0.0; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                u[idx(j, i, nx)] = -(j as f64);
                v[idx(j, i, nx)] = i as f64;
            }
        }

        let divergence = divergence(&u, &v, nx, ny, 1.0, 1.0);
        let vorticity = vorticity(&u, &v, nx, ny, 1.0, 1.0);
        for j in 1..ny - 1 {
            for i in 1..nx - 1 {
                let index = idx(j, i, nx);
                approx(divergence[index], 0.0, 1e-10);
                approx(vorticity[index], 2.0, 1e-10);
            }
        }
    }

    #[test]
    fn linear_theta_with_uniform_flow_has_zero_frontogenesis() {
        let nx = 6;
        let ny = 6;
        let mut theta = vec![0.0; nx * ny];
        let u = vec![10.0; nx * ny];
        let v = vec![5.0; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                theta[idx(j, i, nx)] = 290.0 + i as f64 + j as f64;
            }
        }

        let frontogenesis = frontogenesis(&theta, &u, &v, nx, ny, 1_000.0, 1_000.0);
        for value in frontogenesis {
            approx(value, 0.0, 1e-10);
        }
    }

    #[test]
    fn nine_point_smoother_preserves_linear_fields() {
        let nx = 7;
        let ny = 7;
        let mut values = vec![0.0; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                values[idx(j, i, nx)] = 2.0 * i as f64 + 3.0 * j as f64;
            }
        }

        let smoothed = smooth_n_point(&values, nx, ny, 9, 1);
        for j in 1..ny - 1 {
            for i in 1..nx - 1 {
                let index = idx(j, i, nx);
                approx(smoothed[index], values[index], 1e-10);
            }
        }
    }

    #[test]
    fn advection_matches_linear_scalar_gradient() {
        let nx = 5;
        let ny = 5;
        let dx = 2.0;
        let dy = 4.0;
        let mut scalar = vec![0.0; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                scalar[idx(j, i, nx)] = 3.0 * i as f64 + 5.0 * j as f64;
            }
        }
        let u = vec![10.0; nx * ny];
        let v = vec![5.0; nx * ny];
        let advection = advection(&scalar, &u, &v, nx, ny, dx, dy);
        for value in advection {
            approx(value, -(10.0 * 1.5 + 5.0 * 1.25), 1e-10);
        }
    }

    #[test]
    fn wind_speed_matches_pythagorean_magnitude() {
        let speed = wind_speed(&[3.0, 0.0], &[4.0, 5.0]);
        approx(speed[0], 5.0, 1e-10);
        approx(speed[1], 5.0, 1e-10);
    }

    #[test]
    fn wind_direction_uses_meteorological_from_direction() {
        let mut u = sample_scalar_field("UGRD", "U-Component of Wind", "m/s", "850 mb");
        let mut v = sample_scalar_field("VGRD", "V-Component of Wind", "m/s", "850 mb");
        u.values = vec![
            3.0, 0.0, -4.0, //
            0.0, 0.0, 0.0, //
            1.0, -1.0, 0.0,
        ];
        v.values = vec![
            4.0, 5.0, 0.0, //
            0.0, 0.0, -2.0, //
            -1.0, 1.0, 0.0,
        ];

        let speed = wind_speed_field(&u, &v).expect("wind speed should succeed");
        let direction = wind_direction_field(&u, &v).expect("wind direction should succeed");

        approx(speed.values[0] as f64, 5.0, 1e-10);
        approx(direction.values[0] as f64, 216.869_897_645_844_02, 1e-5);
        approx(direction.values[1] as f64, 180.0, 1e-10);
        approx(direction.values[2] as f64, 90.0, 1e-10);
        approx(direction.values[4] as f64, 0.0, 1e-10);
        approx(direction.values[5] as f64, 0.0, 1e-10);
        assert_eq!(speed.metadata.short_name, "WSPD");
        assert_eq!(direction.metadata.short_name, "WDIR");
        assert_eq!(direction.metadata.units, "degrees");
    }

    #[test]
    fn relative_humidity_saturates_when_temperature_equals_dewpoint() {
        let rh = relative_humidity_from_dewpoint(&[20.0, 0.0], &[20.0, 0.0]);
        approx(rh[0], 100.0, 1e-6);
        approx(rh[1], 100.0, 1e-6);
    }

    #[test]
    fn anomaly_field_removes_domain_mean() {
        let field = sample_scalar_field("TMP", "Temperature", "K", "surface");
        let anomaly = anomaly_field(&field).expect("anomaly field should succeed");
        let stats = field_stats(&anomaly).expect("anomaly should remain finite");
        approx(stats.mean_value, 0.0, 1e-6);
    }

    #[test]
    fn thickness_field_subtracts_lower_height_from_upper_height() {
        let lower = sample_scalar_field("HGT", "Geopotential Height", "gpm", "1000 mb");
        let mut upper = lower.clone();
        upper.metadata.level.description = "500 mb".to_string();
        upper.metadata.level.value = Some(500.0);
        upper.values.iter_mut().for_each(|value| *value += 5400.0);

        let thickness = thickness_field(&lower, &upper).expect("thickness should succeed");
        let stats = field_stats(&thickness).expect("thickness should remain finite");
        approx(stats.min_value as f64, 5400.0, 1e-6);
        approx(stats.max_value as f64, 5400.0, 1e-6);
    }

    #[test]
    fn lapse_rate_field_uses_temperature_drop_per_kilometer() {
        let lower_temperature = sample_scalar_field("TMP", "Temperature", "K", "850 mb");
        let mut upper_temperature = lower_temperature.clone();
        upper_temperature.metadata.level.description = "500 mb".to_string();
        upper_temperature.metadata.level.value = Some(500.0);
        upper_temperature
            .values
            .iter_mut()
            .for_each(|value| *value -= 18.0);

        let mut lower_height = sample_scalar_field("HGT", "Geopotential Height", "m", "850 mb");
        lower_height
            .values
            .iter_mut()
            .for_each(|value| *value = 1_500.0);
        let mut upper_height = lower_height.clone();
        upper_height.metadata.level.description = "500 mb".to_string();
        upper_height.metadata.level.value = Some(500.0);
        upper_height
            .values
            .iter_mut()
            .for_each(|value| *value = 5_000.0);

        let lapse_rate = lapse_rate_field(
            &lower_temperature,
            &upper_temperature,
            &lower_height,
            &upper_height,
        )
        .expect("lapse rate should succeed");
        let stats = field_stats(&lapse_rate).expect("lapse rate should remain finite");
        approx(stats.min_value as f64, 5.142857, 1e-5);
        approx(stats.max_value as f64, 5.142857, 1e-5);
    }

    #[test]
    fn five_point_smoother_matches_known_center_weighting() {
        let nx = 5;
        let ny = 5;
        let mut values = vec![0.0; nx * ny];
        values[idx(2, 2, nx)] = 10.0;

        let smoothed = smooth_n_point(&values, nx, ny, 5, 1);
        approx(smoothed[idx(2, 2, nx)], 5.0, 1e-10);
        approx(smoothed[idx(2, 1, nx)], 1.25, 1e-10);
        approx(smoothed[idx(1, 2, nx)], 1.25, 1e-10);
    }

    #[test]
    fn five_point_smoother_propagates_nan_and_preserves_edges() {
        let nx = 5;
        let ny = 5;
        let mut values = vec![4.0; nx * ny];
        values[idx(2, 2, nx)] = f64::NAN;
        values[idx(0, 0, nx)] = 11.0;

        let smoothed = smooth_n_point(&values, nx, ny, 5, 1);
        assert!(smoothed[idx(2, 2, nx)].is_nan());
        assert!(smoothed[idx(2, 1, nx)].is_nan());
        approx(smoothed[idx(0, 0, nx)], 11.0, 1e-10);
    }

    #[test]
    fn pressure_level_frontogenesis_converts_temperature_to_theta() {
        let nx = 6;
        let ny = 6;
        let dx = 1_000.0;
        let dy = 1_000.0;
        let mut temperature = vec![0.0; nx * ny];
        let mut theta = vec![0.0; nx * ny];
        let u = vec![8.0; nx * ny];
        let v = vec![4.0; nx * ny];
        let pressure_hpa = 850.0_f64;
        let theta_factor = (1000.0_f64 / pressure_hpa).powf(KAPPA);

        for j in 0..ny {
            for i in 0..nx {
                let value = 280.0 + i as f64 + 2.0 * j as f64;
                temperature[idx(j, i, nx)] = value;
                theta[idx(j, i, nx)] = value * theta_factor;
            }
        }

        let from_temperature = frontogenesis(&temperature, &u, &v, nx, ny, dx, dy);
        let from_theta = frontogenesis(&theta, &u, &v, nx, ny, dx, dy);
        for (temperature_value, theta_value) in from_temperature.iter().zip(&from_theta) {
            approx(*theta_value, *temperature_value * theta_factor, 1e-10);
        }
    }

    #[test]
    fn frontogenesis_field_rejects_raw_temperature_inputs() {
        let fields = decode_pressure_fixture_fields();
        let temperature = select_field(&fields, "TMP");
        let u_wind = select_field(&fields, "UGRD");
        let v_wind = select_field(&fields, "VGRD");

        let error = frontogenesis_field(temperature, u_wind, v_wind)
            .expect_err("raw temperature should not satisfy theta frontogenesis");
        assert!(
            error
                .to_string()
                .contains("is not a potential-temperature field"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn vorticity_field_rejects_non_wind_inputs() {
        let fields = decode_pressure_fixture_fields();
        let temperature = select_field(&fields, "TMP");
        let v_wind = select_field(&fields, "VGRD");

        let error = vorticity_field(temperature, v_wind)
            .expect_err("temperature should not be accepted as a wind component");
        assert!(
            error
                .to_string()
                .contains("is not the expected UGRD wind component"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn fixture_backed_vorticity_and_theta_frontogenesis_are_finite() {
        let fields = decode_pressure_fixture_fields();
        let height_1000 = select_field_at_level(&fields, "HGT", "1000 mb");
        let height_500 = select_field_at_level(&fields, "HGT", "500 mb");
        let temperature_850 = select_field_at_level(&fields, "TMP", "850 mb");
        let temperature_1000 = select_field_at_level(&fields, "TMP", "1000 mb");
        let temperature_500 = select_field_at_level(&fields, "TMP", "500 mb");
        let dewpoint_850 = select_field_at_level(&fields, "DPT", "850 mb");
        let u_wind_850 = select_field_at_level(&fields, "UGRD", "850 mb");
        let v_wind_850 = select_field_at_level(&fields, "VGRD", "850 mb");

        let vorticity = vorticity_field(u_wind_850, v_wind_850).expect("vorticity should succeed");
        let smoothed_vorticity =
            smooth_n_point_field(&vorticity, 9, 1).expect("smoothing should succeed");
        let theta =
            potential_temperature_field(temperature_850).expect("theta conversion should work");
        let frontogenesis =
            pressure_level_frontogenesis_field(temperature_850, u_wind_850, v_wind_850)
                .expect("pressure-level frontogenesis should succeed");
        let wind_speed =
            wind_speed_field(u_wind_850, v_wind_850).expect("wind speed should succeed");
        let wind_direction =
            wind_direction_field(u_wind_850, v_wind_850).expect("wind direction should succeed");
        let relative_humidity =
            relative_humidity_field(temperature_850, dewpoint_850).expect("rh should succeed");
        let thickness = thickness_field(height_1000, height_500).expect("thickness should succeed");
        let lapse_rate =
            lapse_rate_field(temperature_1000, temperature_500, height_1000, height_500)
                .expect("lapse rate should succeed");

        assert_eq!(vorticity.grid, u_wind_850.grid);
        assert_eq!(frontogenesis.grid, temperature_850.grid);

        let vort_stats = field_stats(&vorticity).expect("vorticity should contain finite values");
        let smooth_stats =
            field_stats(&smoothed_vorticity).expect("smoothed vorticity should be finite");
        let fronto_stats =
            field_stats(&frontogenesis).expect("frontogenesis should contain finite values");
        let wspd_stats = field_stats(&wind_speed).expect("wind speed should contain finite values");
        let wdir_stats =
            field_stats(&wind_direction).expect("wind direction should contain finite values");
        let rh_stats = field_stats(&relative_humidity)
            .expect("relative humidity should contain finite values");
        let thk_stats = field_stats(&thickness).expect("thickness should contain finite values");
        let lapse_stats =
            field_stats(&lapse_rate).expect("lapse rate should contain finite values");

        assert!(vort_stats.max_value.is_finite());
        assert!(fronto_stats.max_value.is_finite());
        assert!(smooth_stats.max_value.abs() <= vort_stats.max_value.abs() + 1e-6);
        assert!(wspd_stats.max_value.is_finite());
        assert!(wdir_stats.max_value.is_finite());
        assert!(rh_stats.max_value.is_finite());
        assert!(thk_stats.max_value.is_finite());
        assert!(lapse_stats.max_value.is_finite());
        assert_eq!(vorticity.metadata.short_name, "VORT");
        assert_eq!(smoothed_vorticity.metadata.short_name, "SM9");
        assert_eq!(theta.metadata.short_name, "THETA");
        assert_eq!(frontogenesis.metadata.short_name, "FGEN");
        assert_eq!(frontogenesis.metadata.parameter, "Petterssen frontogenesis");
        assert_eq!(wind_speed.metadata.short_name, "WSPD");
        assert_eq!(wind_direction.metadata.short_name, "WDIR");
        assert_eq!(relative_humidity.metadata.short_name, "RH");
        assert_eq!(thickness.metadata.short_name, "THICK");
        assert_eq!(lapse_rate.metadata.short_name, "LRATE");
    }

    fn sample_scalar_field(
        short_name: &str,
        parameter: &str,
        units: &str,
        level_description: &str,
    ) -> Field2D {
        let cycle = Utc
            .with_ymd_and_hms(2024, 4, 1, 0, 0, 0)
            .single()
            .expect("valid cycle");
        Field2D {
            metadata: FieldMetadata {
                short_name: short_name.to_string(),
                parameter: parameter.to_string(),
                units: units.to_string(),
                level: wx_types::LevelMetadata {
                    code: if level_description == "surface" {
                        1
                    } else {
                        100
                    },
                    description: level_description.to_string(),
                    value: if level_description.ends_with("mb") {
                        level_description
                            .split_whitespace()
                            .next()
                            .and_then(|value| value.parse::<f64>().ok())
                    } else {
                        None
                    },
                    units: if level_description.ends_with("mb") {
                        "hPa".to_string()
                    } else {
                        "surface".to_string()
                    },
                },
                source: wx_types::SourceMetadata {
                    provider: "fixture".to_string(),
                    model: "hrrr".to_string(),
                    product: "prs".to_string(),
                },
                run: wx_types::RunMetadata {
                    cycle,
                    forecast_hour: 0,
                },
                valid: wx_types::ValidTimeMetadata {
                    reference_time: cycle,
                    valid_time: cycle,
                },
            },
            grid: wx_types::GridSpec {
                nx: 3,
                ny: 3,
                projection: wx_types::ProjectionKind::LatitudeLongitude,
                coordinates: wx_types::CoordinateMetadata {
                    lat1: 0.0,
                    lon1: 0.0,
                    lat2: 0.0,
                    lon2: 0.0,
                    dx: 1_000.0,
                    dy: 1_000.0,
                },
                scan_mode: 0,
            },
            values: vec![
                1.0, 2.0, 3.0, //
                4.0, 5.0, 6.0, //
                7.0, 8.0, 9.0,
            ],
        }
    }
}
