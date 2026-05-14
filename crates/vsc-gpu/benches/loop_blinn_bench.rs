//! Loop-Blinn Optimization Benchmarks
//!
//! Measures the performance impact of Phase I-1/I-2 Loop-Blinn curve rendering.
//!
//! ## Benchmarks
//!
//! 1. **Tessellation Time**: Compares CPU time for:
//!    - (a) lyon `tessellate_path()` only (baseline)
//!    - (b) `tessellate_quadratic_beziers()` + lyon interior fill (I-1)
//!    - (c) `tessellate_cubic_beziers()` + lyon interior fill (I-2)
//!
//! 2. **Vertex Count**: Compares total vertex output for same input path.
//!
//! ## Running
//!
//! ```bash
//! cargo bench -p vsc-gpu --bench loop_blinn_bench
//! ```
//!
//! ## Baseline Results (2026-05-12, Phase I-1)
//!
//! | Curves | Lyon Time | LB+Interior | Speedup | Vertex Reduction |
//! |--------|-----------|-------------|---------|------------------|
//! | 10     | 146 µs    | 28 µs       | 5.2x    | 3.79x            |
//! | 50     | 917 µs    | 115 µs      | 8.0x    | 3.96x            |
//! | 100    | 2.19 ms   | 217 µs      | 10.1x   | 3.98x            |
//! | 200    | 5.93 ms   | 431 µs      | 13.8x   | 3.99x            |
//!
//! See `docs/BENCHMARK_BASELINE.md` for full analysis.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use vsc_core::{PathCommand, Rational};
use vsc_gpu::loop_blinn::{tessellate_cubic_beziers, tessellate_quadratic_beziers};
use vsc_gpu::tessellation::tessellate_path;
use vsc_gpu::FillStyle;

/// Generate a path with N quadratic Bezier curves forming a wavy line.
///
/// Creates a path that starts at origin and adds N QuadTo segments,
/// each forming a small arc. This simulates glyph-like curve density.
fn generate_quad_path(num_curves: usize) -> Vec<PathCommand> {
    let mut commands = Vec::with_capacity(num_curves + 2);

    // Start at origin
    commands.push(PathCommand::MoveTo {
        x: Rational::from_int(0),
        y: Rational::from_int(100),
    });

    // Add N quadratic curves forming a wavy pattern
    for i in 0..num_curves {
        let base_x = (i as i64) * 20;
        let direction = if i % 2 == 0 { 1 } else { -1 };

        commands.push(PathCommand::QuadTo {
            x1: Rational::from_int(base_x + 10),
            y1: Rational::from_int(100 + direction * 50), // Control point above/below
            x: Rational::from_int(base_x + 20),
            y: Rational::from_int(100),
        });
    }

    // Close the path to form a filled shape
    commands.push(PathCommand::LineTo {
        x: Rational::from_int((num_curves as i64) * 20),
        y: Rational::from_int(0),
    });
    commands.push(PathCommand::LineTo {
        x: Rational::from_int(0),
        y: Rational::from_int(0),
    });
    commands.push(PathCommand::Close);

    commands
}

/// Generate a path with N cubic Bezier curves forming a wavy line.
///
/// Creates a path that starts at origin and adds N CubicTo segments,
/// each forming an S-curve. This simulates typical font/vector curve density.
fn generate_cubic_path(num_curves: usize) -> Vec<PathCommand> {
    let mut commands = Vec::with_capacity(num_curves + 4);

    // Start at origin
    commands.push(PathCommand::MoveTo {
        x: Rational::from_int(0),
        y: Rational::from_int(100),
    });

    // Add N cubic curves forming a wavy pattern (S-curves)
    for i in 0..num_curves {
        let base_x = (i as i64) * 30;
        let direction = if i % 2 == 0 { 1 } else { -1 };

        commands.push(PathCommand::CubicTo {
            x1: Rational::from_int(base_x + 10),
            y1: Rational::from_int(100 + direction * 60), // First control point
            x2: Rational::from_int(base_x + 20),
            y2: Rational::from_int(100 - direction * 60), // Second control point
            x: Rational::from_int(base_x + 30),
            y: Rational::from_int(100),
        });
    }

    // Close the path to form a filled shape
    commands.push(PathCommand::LineTo {
        x: Rational::from_int((num_curves as i64) * 30),
        y: Rational::from_int(0),
    });
    commands.push(PathCommand::LineTo {
        x: Rational::from_int(0),
        y: Rational::from_int(0),
    });
    commands.push(PathCommand::Close);

    commands
}

/// Benchmark: Tessellation time comparison (quadratic).
///
/// Compares CPU time for:
/// - (a) lyon-only path (baseline)
/// - (b) Loop-Blinn + lyon interior
fn bench_tessellation_time(c: &mut Criterion) {
    let mut group = c.benchmark_group("tessellation_time_quad");

    for num_curves in [10, 50, 100, 200] {
        let path = generate_quad_path(num_curves);
        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        // (a) Lyon-only baseline
        group.bench_with_input(
            BenchmarkId::new("lyon_only", num_curves),
            &(&path, &fill),
            |b, (path, fill)| {
                b.iter(|| {
                    let result = tessellate_path(black_box(*path), Some(black_box(*fill)));
                    black_box(result)
                });
            },
        );

        // (b) Loop-Blinn + lyon interior
        group.bench_with_input(
            BenchmarkId::new("loop_blinn_plus_interior", num_curves),
            &(&path, &fill),
            |b, (path, fill)| {
                b.iter(|| {
                    // Step 1: Loop-Blinn tessellation (extracts curves, generates interior commands)
                    let lb_output = tessellate_quadratic_beziers(black_box(*path));

                    // Step 2: Lyon tessellation of interior (with QuadTo -> LineTo)
                    let interior_result =
                        tessellate_path(black_box(&lb_output.interior_commands), Some(black_box(*fill)));

                    black_box((lb_output, interior_result))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Tessellation time comparison (cubic - I-2).
///
/// Compares CPU time for:
/// - (a) lyon-only path (baseline)
/// - (b) Loop-Blinn cubic + lyon interior
fn bench_tessellation_time_cubic(c: &mut Criterion) {
    let mut group = c.benchmark_group("tessellation_time_cubic");

    for num_curves in [10, 50, 100, 200] {
        let path = generate_cubic_path(num_curves);
        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        // (a) Lyon-only baseline
        group.bench_with_input(
            BenchmarkId::new("lyon_only", num_curves),
            &(&path, &fill),
            |b, (path, fill)| {
                b.iter(|| {
                    let result = tessellate_path(black_box(*path), Some(black_box(*fill)));
                    black_box(result)
                });
            },
        );

        // (b) Loop-Blinn cubic + lyon interior
        group.bench_with_input(
            BenchmarkId::new("loop_blinn_cubic_plus_interior", num_curves),
            &(&path, &fill),
            |b, (path, fill)| {
                b.iter(|| {
                    // Step 1: Loop-Blinn cubic tessellation (extracts curves, generates interior commands)
                    let lb_output = tessellate_cubic_beziers(black_box(*path));

                    // Step 2: Lyon tessellation of interior (with CubicTo -> LineTo)
                    let interior_result =
                        tessellate_path(black_box(&lb_output.interior_commands), Some(black_box(*fill)));

                    black_box((lb_output, interior_result))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Vertex count comparison (quadratic).
///
/// Reports vertex counts for same input to quantify triangle reduction.
fn bench_vertex_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("vertex_count_quad");

    for num_curves in [10, 50, 100, 200] {
        let path = generate_quad_path(num_curves);
        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        // (a) Lyon-only vertex count
        group.bench_with_input(
            BenchmarkId::new("lyon_only_vertices", num_curves),
            &(&path, &fill),
            |b, (path, fill)| {
                b.iter(|| {
                    let result = tessellate_path(*path, Some(*fill)).unwrap();
                    black_box(result.vertices.len())
                });
            },
        );

        // (b) Loop-Blinn + interior vertex count
        group.bench_with_input(
            BenchmarkId::new("loop_blinn_total_vertices", num_curves),
            &(&path, &fill),
            |b, (path, fill)| {
                b.iter(|| {
                    let lb_output = tessellate_quadratic_beziers(*path);
                    let interior_result =
                        tessellate_path(&lb_output.interior_commands, Some(*fill)).unwrap();

                    // Total vertices = Loop-Blinn curve vertices + interior vertices
                    let total = lb_output.vertices.len() + interior_result.vertices.len();
                    black_box(total)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Vertex count comparison (cubic - I-2).
///
/// Reports vertex counts for same input to quantify triangle reduction.
fn bench_vertex_count_cubic(c: &mut Criterion) {
    let mut group = c.benchmark_group("vertex_count_cubic");

    for num_curves in [10, 50, 100, 200] {
        let path = generate_cubic_path(num_curves);
        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        // (a) Lyon-only vertex count
        group.bench_with_input(
            BenchmarkId::new("lyon_only_vertices", num_curves),
            &(&path, &fill),
            |b, (path, fill)| {
                b.iter(|| {
                    let result = tessellate_path(*path, Some(*fill)).unwrap();
                    black_box(result.vertices.len())
                });
            },
        );

        // (b) Loop-Blinn cubic + interior vertex count
        group.bench_with_input(
            BenchmarkId::new("loop_blinn_cubic_total_vertices", num_curves),
            &(&path, &fill),
            |b, (path, fill)| {
                b.iter(|| {
                    let lb_output = tessellate_cubic_beziers(*path);
                    let interior_result =
                        tessellate_path(&lb_output.interior_commands, Some(*fill)).unwrap();

                    // Total vertices = Loop-Blinn curve vertices + interior vertices
                    let total = lb_output.vertices.len() + interior_result.vertices.len();
                    black_box(total)
                });
            },
        );
    }

    group.finish();
}

/// Print detailed vertex count comparison (not a benchmark, just measurement).
fn report_vertex_counts() {
    println!("\n=== Quadratic Bezier (I-1) Vertex Count Comparison ===\n");
    println!(
        "{:>10} | {:>15} | {:>15} | {:>10} | {:>10}",
        "Curves", "Lyon Vertices", "LB+Interior", "Reduction", "Ratio"
    );
    println!("{}", "-".repeat(70));

    for num_curves in [10, 50, 100, 200, 500] {
        let path = generate_quad_path(num_curves);
        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        // Lyon-only
        let lyon_result = tessellate_path(&path, Some(&fill)).unwrap();
        let lyon_vertices = lyon_result.vertices.len();

        // Loop-Blinn + interior
        let lb_output = tessellate_quadratic_beziers(&path);
        let interior_result = tessellate_path(&lb_output.interior_commands, Some(&fill)).unwrap();
        let lb_vertices = lb_output.vertices.len();
        let interior_vertices = interior_result.vertices.len();
        let total_lb = lb_vertices + interior_vertices;

        let reduction = lyon_vertices as i64 - total_lb as i64;
        let ratio = lyon_vertices as f64 / total_lb as f64;

        println!(
            "{:>10} | {:>15} | {:>15} | {:>10} | {:>10.2}x",
            num_curves, lyon_vertices, total_lb, reduction, ratio
        );
    }

    println!("\n=== Cubic Bezier (I-2) Vertex Count Comparison ===\n");
    println!(
        "{:>10} | {:>15} | {:>15} | {:>10} | {:>10}",
        "Curves", "Lyon Vertices", "LB+Interior", "Reduction", "Ratio"
    );
    println!("{}", "-".repeat(70));

    for num_curves in [10, 50, 100, 200, 500] {
        let path = generate_cubic_path(num_curves);
        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        // Lyon-only
        let lyon_result = tessellate_path(&path, Some(&fill)).unwrap();
        let lyon_vertices = lyon_result.vertices.len();

        // Loop-Blinn cubic + interior
        let lb_output = tessellate_cubic_beziers(&path);
        let interior_result = tessellate_path(&lb_output.interior_commands, Some(&fill)).unwrap();
        let lb_vertices = lb_output.vertices.len();
        let interior_vertices = interior_result.vertices.len();
        let total_lb = lb_vertices + interior_vertices;

        let reduction = lyon_vertices as i64 - total_lb as i64;
        let ratio = if total_lb > 0 {
            lyon_vertices as f64 / total_lb as f64
        } else {
            0.0
        };

        println!(
            "{:>10} | {:>15} | {:>15} | {:>10} | {:>10.2}x",
            num_curves, lyon_vertices, total_lb, reduction, ratio
        );
    }

    println!("\n=== Quadratic Triangle Count Comparison ===\n");
    println!(
        "{:>10} | {:>15} | {:>15} | {:>10}",
        "Curves", "Lyon Triangles", "LB+Interior", "Reduction"
    );
    println!("{}", "-".repeat(55));

    for num_curves in [10, 50, 100, 200, 500] {
        let path = generate_quad_path(num_curves);
        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        // Lyon-only
        let lyon_result = tessellate_path(&path, Some(&fill)).unwrap();
        let lyon_triangles = lyon_result.indices.len() / 3;

        // Loop-Blinn + interior
        let lb_output = tessellate_quadratic_beziers(&path);
        let interior_result = tessellate_path(&lb_output.interior_commands, Some(&fill)).unwrap();
        let lb_triangles = lb_output.indices.len() / 3;
        let interior_triangles = interior_result.indices.len() / 3;
        let total_lb_triangles = lb_triangles + interior_triangles;

        let reduction = lyon_triangles as i64 - total_lb_triangles as i64;

        println!(
            "{:>10} | {:>15} | {:>15} | {:>10}",
            num_curves, lyon_triangles, total_lb_triangles, reduction
        );
    }

    println!("\n=== Cubic Triangle Count Comparison ===\n");
    println!(
        "{:>10} | {:>15} | {:>15} | {:>10}",
        "Curves", "Lyon Triangles", "LB+Interior", "Reduction"
    );
    println!("{}", "-".repeat(55));

    for num_curves in [10, 50, 100, 200, 500] {
        let path = generate_cubic_path(num_curves);
        let fill = FillStyle::Solid {
            rgba: [255, 0, 0, 255],
        };

        // Lyon-only
        let lyon_result = tessellate_path(&path, Some(&fill)).unwrap();
        let lyon_triangles = lyon_result.indices.len() / 3;

        // Loop-Blinn cubic + interior
        let lb_output = tessellate_cubic_beziers(&path);
        let interior_result = tessellate_path(&lb_output.interior_commands, Some(&fill)).unwrap();
        let lb_triangles = lb_output.indices.len() / 3;
        let interior_triangles = interior_result.indices.len() / 3;
        let total_lb_triangles = lb_triangles + interior_triangles;

        let reduction = lyon_triangles as i64 - total_lb_triangles as i64;

        println!(
            "{:>10} | {:>15} | {:>15} | {:>10}",
            num_curves, lyon_triangles, total_lb_triangles, reduction
        );
    }

    println!();
}

/// Custom benchmark that also prints the vertex count report.
fn bench_with_report(c: &mut Criterion) {
    // Print vertex count report first
    report_vertex_counts();

    // Run quadratic benchmarks (I-1)
    bench_tessellation_time(c);
    bench_vertex_count(c);

    // Run cubic benchmarks (I-2)
    bench_tessellation_time_cubic(c);
    bench_vertex_count_cubic(c);
}

criterion_group!(benches, bench_with_report);
criterion_main!(benches);
