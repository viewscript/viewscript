//! ViewScript CLI (`vsc`)
//!
//! The primary interface for LLMs to operate on ViewScript projects.
//! All output is machine-readable JSON for LLM consumption.

use clap::{Parser, Subcommand};
use std::process::ExitCode;

mod commands;

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
        /// Target renderer (canvaskit, webgl, svg)
        #[arg(short, long, default_value = "canvaskit")]
        target: String,

        /// Output directory
        #[arg(short, long, default_value = "dist")]
        outdir: String,
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

    /// Get current project status
    Status,

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init { name } => commands::init(name),
        Commands::ApiSearch { query, limit } => commands::api_search(&query, limit),
        Commands::CheckWhere { entity_id } => commands::check_where(entity_id),
        Commands::CheckWhen { constraint_id } => commands::check_when(constraint_id),
        Commands::AddObject { object_type, position } => {
            commands::add_object(&object_type, position.as_deref())
        }
        Commands::AddConstraint {
            target,
            component,
            relation,
            term,
            intent,
        } => commands::add_constraint(target, &component, &relation, &term, intent.as_deref()),
        Commands::Optimize { dry_run } => commands::optimize(dry_run),
        Commands::Build { target, outdir } => commands::build(&target, &outdir),
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
        Commands::Status => commands::status(),
        Commands::RunCommand {
            command_file,
            args,
            intent,
        } => commands::run_command(&command_file, &args, intent.as_deref()),
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
