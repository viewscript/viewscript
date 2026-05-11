//! ViewScript Project Configuration (`vsconfig.json`)
//!
//! This module defines the schema for project-level configuration,
//! including resolution strategy weights and telemetry settings.

use crate::collision::ResolutionStrategyWeights;
use crate::telemetry::TelemetryConfig;
use serde::{Deserialize, Serialize};

/// The root configuration structure for a ViewScript project.
/// Stored as `vsconfig.json` in the project root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VsConfig {
    /// Schema version for forward compatibility.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    /// Project metadata.
    pub project: ProjectConfig,

    /// Entry point configuration.
    pub entry: EntryConfig,

    /// Logical viewport dimensions (initial constraints).
    pub viewport: ViewportConfig,

    /// Resolution strategy weights (overridable).
    #[serde(default)]
    pub resolution_strategy_weights: ResolutionStrategyWeights,

    /// Telemetry configuration (opt-in).
    #[serde(default)]
    pub telemetry: TelemetryConfig,

    /// Build configuration.
    #[serde(default)]
    pub build: BuildConfig,
}

fn default_schema_version() -> u32 { 1 }

/// Project metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Project name.
    pub name: String,

    /// Project version (semver).
    #[serde(default = "default_version")]
    pub version: String,

    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_version() -> String { "0.1.0".to_string() }

/// Entry point configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryConfig {
    /// Path to the root .vs file.
    pub main: String,
}

/// Logical viewport dimensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewportConfig {
    /// Logical width in units (not pixels).
    pub width: f64,

    /// Logical height in units (not pixels).
    pub height: f64,

    /// Units per pixel for rendering (affects epsilon visibility threshold).
    #[serde(default = "default_units_per_pixel")]
    pub units_per_pixel: f64,

    /// Time origin (T=0) interpretation.
    #[serde(default)]
    pub time_origin: TimeOrigin,
}

fn default_units_per_pixel() -> f64 { 1.0 }

/// How T=0 is interpreted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeOrigin {
    /// T=0 is the initial render (static content).
    #[default]
    InitialRender,

    /// T=0 is page load (for animations).
    PageLoad,

    /// T=0 is a specific timestamp.
    Timestamp { iso8601: String },
}

/// Build configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Target renderer.
    #[serde(default = "default_target")]
    pub target: String,

    /// Output directory.
    #[serde(default = "default_outdir")]
    pub outdir: String,

    /// Enable chunk splitting for progressive loading.
    #[serde(default)]
    pub chunk_splitting: bool,

    /// Minify IR output.
    #[serde(default = "default_minify")]
    pub minify: bool,
}

fn default_target() -> String { "canvaskit".to_string() }
fn default_outdir() -> String { "dist".to_string() }
fn default_minify() -> bool { true }

impl Default for VsConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            project: ProjectConfig {
                name: "untitled".to_string(),
                version: "0.1.0".to_string(),
                description: None,
            },
            entry: EntryConfig {
                main: "main.vs".to_string(),
            },
            viewport: ViewportConfig {
                width: 1920.0,
                height: 1080.0,
                units_per_pixel: 1.0,
                time_origin: TimeOrigin::default(),
            },
            resolution_strategy_weights: ResolutionStrategyWeights::default(),
            telemetry: TelemetryConfig::default(),
            build: BuildConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Rational;

    #[test]
    fn test_config_serialization() {
        let config = VsConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();

        assert!(json.contains("resolution_strategy_weights"));
        assert!(json.contains("telemetry"));

        // Verify round-trip
        let parsed: VsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.resolution_strategy_weights.deletion, Rational::from_int(1000));
    }

    #[test]
    fn test_custom_weights() {
        // Note: JSON uses rational string format "numerator/denominator"
        let json = r#"{
            "schema_version": 1,
            "project": { "name": "test" },
            "entry": { "main": "main.vs" },
            "viewport": { "width": 800, "height": 600 },
            "resolution_strategy_weights": {
                "deletion": "500/1",
                "relation_change": "50/1"
            }
        }"#;

        let config: VsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.resolution_strategy_weights.deletion, Rational::from_int(500));
        assert_eq!(config.resolution_strategy_weights.relation_change, Rational::from_int(50));
        // Defaults still apply for unspecified fields
        assert_eq!(config.resolution_strategy_weights.constant_modification, Rational::from_int(10));
    }
}
