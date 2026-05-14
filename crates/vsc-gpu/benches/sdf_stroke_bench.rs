//! SDF Stroke Optimization Benchmarks
//!
//! Measures the performance impact of Phase I-3/I-4 SDF stroke rendering.
//!
//! ## Benchmarks
//!
//! 1. **Tessellation Time**: Compares CPU time for:
//!    - (a) lyon `tessellate_path_stroke()` (baseline)
//!    - (b) SDF `tessellate_stroke_segments()` (quadratic - I-3)
//!    - (c) SDF `tessellate_cubic_stroke_segments()` (cubic - I-4)
//!
//! 2. **Vertex Count**: Compares total vertex output for same input path.
//!    - SDF Stroke (quadratic): 4 vertices per segment (fixed)
//!    - SDF Stroke (cubic): 4 vertices per segment (fixed)
//!    - Lyon: Varies based on stroke width and curve complexity
//!
//! ## Running
//!
//! ```bash
//! cargo bench -p vsc-gpu --bench sdf_stroke_bench
//! ```
//!
//! ## Theoretical Analysis
//!
//! For N Bezier segments:
//! - **SDF Stroke**: O(N) with exactly 4N vertices, 6N indices
//! - **Lyon Stroke**: O(N × W) where W depends on stroke width and tessellation tolerance
//!
//! SDF stroke moves complexity from CPU tessellation to GPU fragment shader.
//! - Quadratic (I-3): Cardano's formula for distance
//! - Cubic (I-4): Newton's method for distance

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use vsc_core::{PathCommand, Rational};
use vsc_gpu::sdf_stroke::{tessellate_cubic_stroke_segments, tessellate_stroke_segments};
use vsc_gpu::tessellation::tessellate_path_stroke;
use vsc_gpu::StrokeStyle;

/// Generate a path with N quadratic Bezier curves forming a wavy line.
///
/// Creates an open path (no fill) suitable for stroke rendering.
fn generate_stroke_path(num_curves: usize) -> Vec<PathCommand> {
    let mut commands = Vec::with_capacity(num_curves + 1);

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

    // Open path (no Close) - typical for stroke-only rendering
    commands
}

/// Generate a path with N cubic Bezier curves forming a wavy line.
///
/// Creates an open path (no fill) suitable for stroke rendering.
fn generate_cubic_stroke_path(num_curves: usize) -> Vec<PathCommand> {
    let mut commands = Vec::with_capacity(num_curves + 1);

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

    // Open path (no Close) - typical for stroke-only rendering
    commands
}

/// Create a stroke style for benchmarking.
fn create_stroke_style(width: f32) -> StrokeStyle {
    StrokeStyle {
        rgba: [0, 0, 0, 255],
        width: Rational::from_int(width as i64),
        line_cap: vsc_gpu::LineCap::Butt,
        line_join: vsc_gpu::LineJoin::Miter,
        dash_array: None,
    }
}

/// Benchmark: Tessellation time comparison (quadratic - I-3).
///
/// Compares CPU time for:
/// - (a) lyon stroke tessellation (baseline)
/// - (b) SDF stroke tessellation
fn bench_stroke_tessellation_time(c: &mut Criterion) {
    let mut group = c.benchmark_group("stroke_tessellation_time_quad");

    let stroke_width = 4.0f32;
    let stroke_style = create_stroke_style(stroke_width);

    for num_curves in [10, 50, 100, 200] {
        let path = generate_stroke_path(num_curves);

        // (a) Lyon stroke baseline
        group.bench_with_input(
            BenchmarkId::new("lyon_stroke", num_curves),
            &(&path, &stroke_style),
            |b, (path, stroke)| {
                b.iter(|| {
                    let result = tessellate_path_stroke(black_box(*path), black_box(*stroke));
                    black_box(result)
                });
            },
        );

        // (b) SDF stroke
        group.bench_with_input(
            BenchmarkId::new("sdf_stroke", num_curves),
            &(&path, stroke_width),
            |b, (path, width)| {
                b.iter(|| {
                    let result = tessellate_stroke_segments(black_box(*path), black_box(*width));
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Tessellation time comparison (cubic - I-4).
///
/// Compares CPU time for:
/// - (a) lyon stroke tessellation (baseline)
/// - (b) SDF cubic stroke tessellation
fn bench_stroke_tessellation_time_cubic(c: &mut Criterion) {
    let mut group = c.benchmark_group("stroke_tessellation_time_cubic");

    let stroke_width = 4.0f32;
    let stroke_style = create_stroke_style(stroke_width);

    for num_curves in [10, 50, 100, 200] {
        let path = generate_cubic_stroke_path(num_curves);

        // (a) Lyon stroke baseline
        group.bench_with_input(
            BenchmarkId::new("lyon_stroke", num_curves),
            &(&path, &stroke_style),
            |b, (path, stroke)| {
                b.iter(|| {
                    let result = tessellate_path_stroke(black_box(*path), black_box(*stroke));
                    black_box(result)
                });
            },
        );

        // (b) SDF cubic stroke
        group.bench_with_input(
            BenchmarkId::new("sdf_stroke_cubic", num_curves),
            &(&path, stroke_width),
            |b, (path, width)| {
                b.iter(|| {
                    let result =
                        tessellate_cubic_stroke_segments(black_box(*path), black_box(*width));
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Vertex count comparison (quadratic).
///
/// Reports vertex counts for analysis (not timed).
fn bench_vertex_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("stroke_vertex_count_quad");

    let stroke_width = 4.0f32;
    let stroke_style = create_stroke_style(stroke_width);

    for num_curves in [10, 50, 100, 200] {
        let path = generate_stroke_path(num_curves);

        // Lyon vertex count
        let lyon_result = tessellate_path_stroke(&path, &stroke_style);
        let lyon_vertices = lyon_result.map(|t| t.vertices.len()).unwrap_or(0);

        // SDF vertex count
        let sdf_result = tessellate_stroke_segments(&path, stroke_width);
        let sdf_vertices = sdf_result.vertices.len();

        // Benchmark that just returns the counts (for reporting)
        group.bench_with_input(
            BenchmarkId::new("lyon_vertices", num_curves),
            &lyon_vertices,
            |b, &count| {
                b.iter(|| black_box(count));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("sdf_vertices", num_curves),
            &sdf_vertices,
            |b, &count| {
                b.iter(|| black_box(count));
            },
        );

        // Print comparison for analysis
        let reduction = if sdf_vertices > 0 {
            lyon_vertices as f64 / sdf_vertices as f64
        } else {
            0.0
        };
        println!(
            "  Quad {} curves: lyon={} vertices, sdf={} vertices, reduction={:.2}x",
            num_curves, lyon_vertices, sdf_vertices, reduction
        );
    }

    group.finish();
}

/// Benchmark: Vertex count comparison (cubic - I-4).
///
/// Reports vertex counts for analysis (not timed).
fn bench_vertex_count_cubic(c: &mut Criterion) {
    let mut group = c.benchmark_group("stroke_vertex_count_cubic");

    let stroke_width = 4.0f32;
    let stroke_style = create_stroke_style(stroke_width);

    for num_curves in [10, 50, 100, 200] {
        let path = generate_cubic_stroke_path(num_curves);

        // Lyon vertex count
        let lyon_result = tessellate_path_stroke(&path, &stroke_style);
        let lyon_vertices = lyon_result.map(|t| t.vertices.len()).unwrap_or(0);

        // SDF cubic vertex count
        let sdf_result = tessellate_cubic_stroke_segments(&path, stroke_width);
        let sdf_vertices = sdf_result.vertices.len();

        // Benchmark that just returns the counts (for reporting)
        group.bench_with_input(
            BenchmarkId::new("lyon_vertices", num_curves),
            &lyon_vertices,
            |b, &count| {
                b.iter(|| black_box(count));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("sdf_cubic_vertices", num_curves),
            &sdf_vertices,
            |b, &count| {
                b.iter(|| black_box(count));
            },
        );

        // Print comparison for analysis
        let reduction = if sdf_vertices > 0 {
            lyon_vertices as f64 / sdf_vertices as f64
        } else {
            0.0
        };
        println!(
            "  Cubic {} curves: lyon={} vertices, sdf={} vertices, reduction={:.2}x",
            num_curves, lyon_vertices, sdf_vertices, reduction
        );
    }

    group.finish();
}

/// Print detailed comparison analysis.
fn print_analysis() {
    println!("\n=== SDF Stroke Benchmark Analysis (Quadratic - I-3) ===\n");

    let stroke_width = 4.0f32;
    let stroke_style = create_stroke_style(stroke_width);

    println!("| Curves | Lyon Vertices | SDF Vertices | Reduction | Lyon Indices | SDF Indices |");
    println!("|--------|---------------|--------------|-----------|--------------|-------------|");

    for num_curves in [10, 50, 100, 200] {
        let path = generate_stroke_path(num_curves);

        let lyon_result = tessellate_path_stroke(&path, &stroke_style);
        let (lyon_v, lyon_i) = lyon_result
            .map(|t| (t.vertices.len(), t.indices.len()))
            .unwrap_or((0, 0));

        let sdf_result = tessellate_stroke_segments(&path, stroke_width);
        let sdf_v = sdf_result.vertices.len();
        let sdf_i = sdf_result.indices.len();

        let reduction = if sdf_v > 0 {
            lyon_v as f64 / sdf_v as f64
        } else {
            0.0
        };

        println!(
            "| {:>6} | {:>13} | {:>12} | {:>9.2}x | {:>12} | {:>11} |",
            num_curves, lyon_v, sdf_v, reduction, lyon_i, sdf_i
        );
    }

    println!("\nTheoretical SDF (quadratic): 4 vertices/segment, 6 indices/segment");
    println!("SDF moves tessellation cost to GPU fragment shader (Cardano's formula).\n");

    println!("\n=== SDF Stroke Benchmark Analysis (Cubic - I-4) ===\n");

    println!("| Curves | Lyon Vertices | SDF Vertices | Reduction | Lyon Indices | SDF Indices |");
    println!("|--------|---------------|--------------|-----------|--------------|-------------|");

    for num_curves in [10, 50, 100, 200] {
        let path = generate_cubic_stroke_path(num_curves);

        let lyon_result = tessellate_path_stroke(&path, &stroke_style);
        let (lyon_v, lyon_i) = lyon_result
            .map(|t| (t.vertices.len(), t.indices.len()))
            .unwrap_or((0, 0));

        let sdf_result = tessellate_cubic_stroke_segments(&path, stroke_width);
        let sdf_v = sdf_result.vertices.len();
        let sdf_i = sdf_result.indices.len();

        let reduction = if sdf_v > 0 {
            lyon_v as f64 / sdf_v as f64
        } else {
            0.0
        };

        println!(
            "| {:>6} | {:>13} | {:>12} | {:>9.2}x | {:>12} | {:>11} |",
            num_curves, lyon_v, sdf_v, reduction, lyon_i, sdf_i
        );
    }

    println!("\nTheoretical SDF (cubic): 4 vertices/segment, 6 indices/segment");
    println!("SDF moves tessellation cost to GPU fragment shader (Newton's method).\n");
}

criterion_group!(
    benches,
    bench_stroke_tessellation_time,
    bench_vertex_count,
    bench_stroke_tessellation_time_cubic,
    bench_vertex_count_cubic,
);
criterion_main!(benches);

/// Run analysis as a test for quick verification.
#[test]
fn test_print_analysis() {
    print_analysis();
}
