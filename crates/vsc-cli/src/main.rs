//! ViewScript CLI (`vsc`)
//!
//! The primary interface for LLMs to operate on ViewScript projects.
//! All output is machine-readable JSON for LLM consumption.

use clap::{Parser, Subcommand};
use std::process::ExitCode;

mod commands;
pub mod embedded_wasm;

#[derive(Parser)]
#[command(name = "vsc")]
#[command(about = "ViewScript CLI - View Framework for LLMs")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new ViewScript project
    Init {
        /// Project name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Search API functions by natural language query
    ApiSearch {
        /// Natural language query (e.g., "center a square on the screen")
        query: String,

        /// Maximum number of results
        #[arg(short, long, default_value = "5")]
        limit: usize,
    },

    /// Check where an object can be placed (XY constraints)
    CheckWhere {
        /// Entity ID to check
        entity_id: u64,
    },

    /// Check when a constraint is satisfied (T constraints)
    CheckWhen {
        /// Constraint ID to check
        constraint_id: u64,
    },

    /// Add a new object to the scene
    AddObject {
        /// Object type (point, curve, surface)
        object_type: String,

        /// Initial position as JSON
        #[arg(short, long)]
        position: Option<String>,
    },

    /// Add a constraint between entities
    AddConstraint {
        /// Target entity ID
        target: u64,

        /// Component (x, y, z, t)
        component: String,

        /// Relation (eq, lt, le, gt, ge)
        relation: String,

        /// Term as JSON
        term: String,

        /// Natural language intent
        #[arg(short, long)]
        intent: Option<String>,
    },

    /// Optimize the IR by removing redundant constraints
    Optimize {
        /// Dry run (show what would be optimized without applying)
        #[arg(short, long)]
        dry_run: bool,
    },

    /// Build the project for a target renderer
    Build {
        /// Target renderer (wgpu, webgl, svg)
        #[arg(short, long, default_value = "wgpu")]
        target: String,

        /// Output directory
        #[arg(short, long, default_value = "dist")]
        outdir: String,
    },

    /// Start development server with live preview
    Dev {
        /// Target renderer (vs-web)
        #[arg(long, default_value = "vs-web")]
        target: String,

        /// Port number
        #[arg(long, default_value = "8787")]
        port: u16,
    },

    /// Modify an existing constraint on an entity
    PatchConstraint {
        /// Entity ID to modify
        #[arg(long)]
        entity_id: u64,

        /// Component to modify (x, y, width, height, etc.)
        #[arg(long)]
        component: String,

        /// New relation (eq, le, ge)
        #[arg(long)]
        relation: String,

        /// New value (rational, e.g., "100" or "100/1")
        #[arg(long)]
        value: String,

        /// Natural language intent
        #[arg(long)]
        intent: Option<String>,
    },

    /// Add a layout combinator (alias for apply-layout)
    AddLayout {
        /// Layout type (stack_vertical, stack_horizontal)
        layout_type: String,

        /// Instance IDs as JSON array, e.g., "[101, 102, 103]"
        #[arg(long)]
        instances: String,

        /// Anchor point (TL, TR, BL, BR)
        #[arg(long, default_value = "TL")]
        anchor: Option<String>,

        /// Gap between instances (rational, e.g., "16" or "32/2")
        #[arg(long, default_value = "0")]
        gap: Option<String>,

        /// X origin for first instance
        #[arg(long)]
        origin_x: Option<String>,

        /// Y origin for first instance
        #[arg(long)]
        origin_y: Option<String>,

        /// Natural language intent
        #[arg(long)]
        intent: Option<String>,
    },

    /// Add a new entity to the constraint graph (Phase 10)
    AddEntity {
        /// Entity type (text)
        #[arg(long, short = 't')]
        entity_type: String,

        /// Content (for text entities)
        #[arg(long, short = 'c')]
        content: Option<String>,

        /// Font family (for text entities)
        #[arg(long)]
        font_family: Option<String>,

        /// Font size (for text entities)
        #[arg(long)]
        font_size: Option<String>,

        /// Initial X position
        #[arg(long, short = 'x')]
        x: Option<String>,

        /// Initial Y position
        #[arg(long, short = 'y')]
        y: Option<String>,
    },

    /// Add a visual component (RoundedRect, Circle, etc.)
    AddComponent {
        /// Component type (RoundedRect, Circle, Line, Path)
        #[arg(long, short = 't')]
        component_type: String,

        /// X position
        #[arg(long, short = 'x', default_value = "0")]
        x: String,

        /// Y position
        #[arg(long, short = 'y', default_value = "0")]
        y: String,

        /// Width (for RoundedRect)
        #[arg(long, short = 'w', default_value = "100")]
        width: String,

        /// Height (for RoundedRect)
        #[arg(long, default_value = "100")]
        height: String,

        /// Corner radius (for RoundedRect)
        #[arg(long, short = 'r', default_value = "0")]
        radius: String,

        /// Fill: solid color "#ff6b6b" or gradient "linear-gradient(to right, #ff0000, #00ff00)"
        #[arg(long, short = 'f', default_value = "#888888")]
        fill: String,

        /// Stroke: "width:color" format (e.g., "2:#000000")
        #[arg(long, short = 's')]
        stroke: Option<String>,
    },

    /// Update text metrics from Renderer measurement (Phase 10 Q→P bridge)
    UpdateMetrics {
        /// Text entity ID
        #[arg(long)]
        id: u64,

        /// Measured width
        #[arg(long)]
        width: String,

        /// Measured height
        #[arg(long)]
        height: String,
    },

    /// Apply a layout combinator to arrange instances (Phase 13)
    ApplyLayout {
        /// Layout type (stack_vertical, stack_horizontal)
        layout_type: String,

        /// Instance IDs as JSON array, e.g., "[101, 102, 103]"
        #[arg(long)]
        instances: String,

        /// Anchor point (TL, TR, BL, BR)
        #[arg(long, default_value = "TL")]
        anchor: Option<String>,

        /// Gap between instances (rational, e.g., "16" or "32/2")
        #[arg(long, default_value = "0")]
        gap: Option<String>,

        /// X origin for first instance
        #[arg(long)]
        origin_x: Option<String>,

        /// Y origin for first instance
        #[arg(long)]
        origin_y: Option<String>,

        /// Natural language intent
        #[arg(long)]
        intent: Option<String>,
    },

    /// Remove a constraint or layout macro (Phase 13)
    RemoveConstraint {
        /// Constraint ID or layout macro sequence number
        target_id: u64,

        /// Natural language intent
        #[arg(long)]
        intent: Option<String>,
    },

    /// Export OpenAPI schema for LLM agent initialization (Phase 14)
    ExportSchema {
        /// Output format (yaml or json)
        #[arg(long, default_value = "yaml")]
        format: String,
    },

    /// Generate JSON Schema from Rust type definitions (D-16)
    GenerateSchema {
        /// Schema target: buildinfo, constraint, codl, or all (default)
        #[arg(long, default_value = "all")]
        target: String,
    },

    /// Get current project status
    Status,

    /// Search and query objects in the constraint graph
    Search {
        /// Object type filter: constraint, path, control-point, text, gradient, q-variable, derived, all
        #[arg(short = 't', long)]
        object_type: Option<String>,

        /// Filter by entity ID
        #[arg(short = 'e', long)]
        entity_id: Option<u64>,

        /// Filter by component (x, y, width, height, etc.)
        #[arg(short = 'c', long)]
        component: Option<String>,

        /// Constraint satisfaction filter (e.g., "x > 100", "width == 200")
        #[arg(short = 'w', long)]
        where_clause: Option<String>,

        /// Maximum results to return
        #[arg(short = 'l', long, default_value = "100")]
        limit: usize,
    },

    /// Check constraint graph integrity (D-05)
    Check {
        /// Check specific aspects (all if omitted): cycles, rigidity, singularity, types
        #[arg(long)]
        aspect: Option<String>,
    },

    /// Run a CODL command file (Phase 15)
    RunCommand {
        /// Path to .vscmd.yaml file
        command_file: String,

        /// Arguments as JSON object
        #[arg(long)]
        args: String,

        /// Natural language intent
        #[arg(long)]
        intent: Option<String>,
    },

    /// Manage render targets
    Target {
        #[command(subcommand)]
        action: TargetAction,
    },

    /// Manage stylesheets (UA stylesheets like vs-style-chrome)
    Style {
        #[command(subcommand)]
        action: StyleAction,
    },

    /// Compile ViewScript to standalone JavaScript (C4.2)
    ///
    /// Reads VsBuildInfo from stdin (JSON), runs solver and tessellation,
    /// and outputs standalone JavaScript to stdout.
    CompileJs {
        /// Read input from stdin (default: true)
        #[arg(long, default_value = "true")]
        stdin: bool,
    },
}

/// Actions for the `target` subcommand.
#[derive(Subcommand)]
enum TargetAction {
    /// Add a render target to the project
    Add {
        /// Target name (e.g., "vs-web")
        name: String,
    },

    /// Remove a render target from the project
    Remove {
        /// Target name to remove
        name: String,
    },

    /// List registered render targets
    List,
}

/// Actions for the `style` subcommand.
#[derive(Subcommand)]
enum StyleAction {
    /// Add a stylesheet to the project (e.g., "vs-style-chrome")
    Add {
        /// Style name (e.g., "vs-style-chrome")
        name: String,
    },

    /// Remove a stylesheet from the project
    Remove {
        /// Style name to remove
        name: String,
    },

    /// List registered stylesheets
    List,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init { name } => commands::init(name),
        Commands::ApiSearch { query, limit } => commands::api_search(&query, limit),
        Commands::CheckWhere { entity_id } => commands::check_where(entity_id),
        Commands::CheckWhen { constraint_id } => commands::check_when(constraint_id),
        Commands::AddObject {
            object_type,
            position,
        } => commands::add_object(&object_type, position.as_deref()),
        Commands::AddConstraint {
            target,
            component,
            relation,
            term,
            intent,
        } => commands::add_constraint(target, &component, &relation, &term, intent.as_deref()),
        Commands::Optimize { dry_run } => commands::optimize(dry_run),
        Commands::Build { target, outdir } => commands::build(&target, &outdir),
        Commands::Dev { target, port } => commands::dev(&target, port),
        Commands::PatchConstraint {
            entity_id,
            component,
            relation,
            value,
            intent,
        } => {
            commands::patch_constraint(entity_id, &component, &relation, &value, intent.as_deref())
        }
        Commands::AddLayout {
            layout_type,
            instances,
            anchor,
            gap,
            origin_x,
            origin_y,
            intent,
        } => commands::apply_layout(
            &layout_type,
            &instances,
            anchor.as_deref(),
            gap.as_deref(),
            origin_x.as_deref(),
            origin_y.as_deref(),
            intent.as_deref(),
        ),
        Commands::AddEntity {
            entity_type,
            content,
            font_family,
            font_size,
            x,
            y,
        } => commands::add_entity(
            &entity_type,
            content.as_deref(),
            font_family.as_deref(),
            font_size.as_deref(),
            x.as_deref(),
            y.as_deref(),
        ),
        Commands::AddComponent {
            component_type,
            x,
            y,
            width,
            height,
            radius,
            fill,
            stroke,
        } => commands::add_component(
            &component_type,
            &x,
            &y,
            &width,
            &height,
            &radius,
            &fill,
            stroke.as_deref(),
        ),
        Commands::UpdateMetrics { id, width, height } => {
            commands::update_metrics(id, &width, &height)
        }
        Commands::ApplyLayout {
            layout_type,
            instances,
            anchor,
            gap,
            origin_x,
            origin_y,
            intent,
        } => commands::apply_layout(
            &layout_type,
            &instances,
            anchor.as_deref(),
            gap.as_deref(),
            origin_x.as_deref(),
            origin_y.as_deref(),
            intent.as_deref(),
        ),
        Commands::RemoveConstraint { target_id, intent } => {
            commands::remove_constraint(target_id, intent.as_deref())
        }
        Commands::ExportSchema { format } => commands::export_schema(&format),
        Commands::GenerateSchema { target } => commands::generate_schema(&target),
        Commands::Status => commands::status(),
        Commands::Search {
            object_type,
            entity_id,
            component,
            where_clause,
            limit,
        } => commands::search(
            object_type.as_deref(),
            entity_id,
            component.as_deref(),
            where_clause.as_deref(),
            limit,
        ),
        Commands::Check { aspect } => commands::check(aspect.as_deref()),
        Commands::RunCommand {
            command_file,
            args,
            intent,
        } => commands::run_command(&command_file, &args, intent.as_deref()),
        Commands::Target { action } => match action {
            TargetAction::Add { name } => commands::target_add(&name),
            TargetAction::Remove { name } => commands::target_remove(&name),
            TargetAction::List => commands::target_list(),
        },
        Commands::Style { action } => match action {
            StyleAction::Add { name } => commands::style_add(&name),
            StyleAction::Remove { name } => commands::style_remove(&name),
            StyleAction::List => commands::style_list(),
        },
        Commands::CompileJs { stdin } => commands::compile_js(stdin),
    };

    match result {
        Ok(output) => {
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", serde_json::to_string_pretty(&e).unwrap());
            ExitCode::FAILURE
        }
    }
}
