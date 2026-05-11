//! Telemetry Schema for LLM Repair Suggestion Acceptance Metrics
//!
//! This module defines the opt-in telemetry structures for measuring
//! how well the repair suggestion ordering aligns with LLM behavior.
//!
//! ## Local Knowledge Base for LLM Self-Correction
//!
//! The `.vs-telemetry.jsonl` file serves dual purposes:
//! 1. **Metrics collection**: Track suggestion acceptance rates
//! 2. **RAG knowledge base**: Provide context for LLM self-correction
//!
//! When the CLI detects a collision, it analyzes the local telemetry ledger
//! to identify patterns (e.g., "suggestions of type X are frequently rejected
//! in this project") and injects this context into the repair suggestions.
//!
//! Privacy: All telemetry is local-only by default. No data leaves the machine
//! unless explicitly configured with an endpoint in vsconfig.json.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Telemetry event emitted when an LLM responds to a repair suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairSuggestionEvent {
    /// Anonymous session ID (hashed, not traceable to user).
    pub session_hash: String,

    /// ISO 8601 timestamp.
    pub timestamp: String,

    /// The collision type that triggered the suggestions.
    pub collision_type: String,

    /// Total number of suggestions presented.
    pub suggestions_count: u32,

    /// The outcome of the LLM's decision.
    pub outcome: RepairOutcome,

    /// The weights configuration used (for correlation analysis).
    pub weights_config: WeightsSnapshot,
}

/// The outcome of an LLM's response to repair suggestions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RepairOutcome {
    /// LLM accepted a suggestion by its ID.
    Accepted {
        /// 1-indexed rank of the accepted suggestion (1 = top suggestion).
        accepted_rank: u32,
        /// The composite score of the accepted suggestion.
        accepted_score: f64,
    },

    /// LLM rejected all suggestions and provided a custom resolution.
    CustomResolution {
        /// Number of constraints the LLM manually added/modified.
        manual_changes: u32,
    },

    /// LLM abandoned the operation (e.g., undo, rollback).
    Abandoned,

    /// LLM requested more information before deciding.
    RequestedInfo {
        /// The type of info requested.
        info_type: String,
    },
}

/// Snapshot of the weights configuration for correlation with outcomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightsSnapshot {
    pub deletion: f64,
    pub relation_change: f64,
    pub constant_modification: f64,
    pub relative_error: f64,
}

/// Aggregated metrics for analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairMetricsAggregate {
    /// Total events in this aggregate.
    pub total_events: u64,

    /// Acceptance rate of Top-1 suggestion: count(accepted_rank=1) / count(Accepted)
    pub top1_acceptance_rate: f64,

    /// Acceptance rate of Top-3 suggestions.
    pub top3_acceptance_rate: f64,

    /// Rate of custom resolutions (LLM didn't use suggestions).
    pub custom_resolution_rate: f64,

    /// Rate of abandoned operations.
    pub abandonment_rate: f64,

    /// Mean accepted rank (lower = better suggestion ordering).
    pub mean_accepted_rank: f64,

    /// Breakdown by collision type.
    pub by_collision_type: Vec<CollisionTypeMetrics>,
}

/// Metrics broken down by collision type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollisionTypeMetrics {
    pub collision_type: String,
    pub event_count: u64,
    pub top1_acceptance_rate: f64,
    pub mean_accepted_rank: f64,
}

/// Telemetry configuration in vsconfig.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Whether telemetry is enabled (default: false, opt-in).
    #[serde(default)]
    pub enabled: bool,

    /// Endpoint URL for telemetry submission (if enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Whether to include weights snapshot in events.
    #[serde(default = "default_include_weights")]
    pub include_weights: bool,

    /// Local-only mode: write events to .vs-telemetry.jsonl instead of sending.
    #[serde(default)]
    pub local_only: bool,
}

fn default_include_weights() -> bool { true }

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            include_weights: true,
            local_only: true,
        }
    }
}

/// Validation metric: Is the Top-1 acceptance rate above threshold?
impl RepairMetricsAggregate {
    /// Check if the suggestion ordering is performing well.
    /// A Top-1 acceptance rate of 0.7 (70%) is considered good.
    pub fn is_ordering_effective(&self, threshold: f64) -> bool {
        self.top1_acceptance_rate >= threshold
    }

    /// Suggest weight adjustments based on observed behavior.
    pub fn suggest_weight_adjustments(&self) -> Option<WeightAdjustment> {
        if self.top1_acceptance_rate < 0.5 {
            // Poor ordering: suggest investigating weights
            Some(WeightAdjustment {
                reason: "Top-1 acceptance rate below 50%".to_string(),
                suggested_action: "Review collision types with lowest acceptance rates".to_string(),
                collision_types_to_review: self
                    .by_collision_type
                    .iter()
                    .filter(|m| m.top1_acceptance_rate < 0.5)
                    .map(|m| m.collision_type.clone())
                    .collect(),
            })
        } else {
            None
        }
    }
}

/// Suggested adjustment to weights based on telemetry analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightAdjustment {
    pub reason: String,
    pub suggested_action: String,
    pub collision_types_to_review: Vec<String>,
}

// =============================================================================
// Local Knowledge Base for LLM Self-Correction (RAG)
// =============================================================================

/// The local knowledge base derived from `.vs-telemetry.jsonl`.
/// This is used to provide contextual hints to the LLM based on
/// historical patterns in this specific project.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalKnowledgeBase {
    /// Total events analyzed.
    pub events_analyzed: u64,

    /// Patterns detected from historical outcomes.
    pub patterns: Vec<BehaviorPattern>,

    /// Per-collision-type statistics.
    pub collision_stats: HashMap<String, CollisionStatistics>,

    /// Recent rejections (for immediate context).
    pub recent_rejections: Vec<RejectionContext>,
}

/// A detected pattern in LLM behavior for this project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorPattern {
    /// Human-readable description of the pattern.
    pub description: String,

    /// Confidence score (0.0 to 1.0).
    pub confidence: f64,

    /// Number of events supporting this pattern.
    pub supporting_events: u64,

    /// Actionable hint for the LLM.
    pub hint: String,
}

/// Statistics for a specific collision type in this project.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollisionStatistics {
    /// Total occurrences.
    pub occurrences: u64,

    /// How often Top-1 was accepted.
    pub top1_accepted: u64,

    /// How often custom resolution was used.
    pub custom_resolutions: u64,

    /// How often the operation was abandoned.
    pub abandonments: u64,

    /// Average rank when accepted (lower = suggestions are good).
    pub avg_accepted_rank: f64,
}

/// Context about a recent rejection for immediate learning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectionContext {
    /// The collision type that was rejected.
    pub collision_type: String,

    /// The suggestion rank that was ultimately chosen (if any).
    pub chosen_rank: Option<u32>,

    /// Brief summary of the rejection pattern.
    pub summary: String,
}

impl LocalKnowledgeBase {
    /// Load the knowledge base from `.vs-telemetry.jsonl`.
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, std::io::Error> {
        use std::io::{BufRead, BufReader};

        let file = std::fs::File::open(path)?;
        let reader = BufReader::new(file);

        let mut events: Vec<RepairSuggestionEvent> = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<RepairSuggestionEvent>(&line) {
                events.push(event);
            }
        }

        Ok(Self::from_events(&events))
    }

    /// Build the knowledge base from a list of events.
    pub fn from_events(events: &[RepairSuggestionEvent]) -> Self {
        let mut kb = Self {
            events_analyzed: events.len() as u64,
            ..Default::default()
        };

        // Aggregate statistics by collision type
        for event in events {
            let stats = kb
                .collision_stats
                .entry(event.collision_type.clone())
                .or_default();

            stats.occurrences += 1;

            match &event.outcome {
                RepairOutcome::Accepted { accepted_rank, .. } => {
                    if *accepted_rank == 1 {
                        stats.top1_accepted += 1;
                    }
                    // Update running average
                    let n = stats.occurrences as f64;
                    stats.avg_accepted_rank =
                        (stats.avg_accepted_rank * (n - 1.0) + *accepted_rank as f64) / n;
                }
                RepairOutcome::CustomResolution { .. } => {
                    stats.custom_resolutions += 1;
                }
                RepairOutcome::Abandoned => {
                    stats.abandonments += 1;
                }
                RepairOutcome::RequestedInfo { .. } => {}
            }
        }

        // Detect patterns
        kb.detect_patterns();

        // Capture recent rejections (last 10 non-top1 acceptances)
        kb.recent_rejections = events
            .iter()
            .rev()
            .filter_map(|e| match &e.outcome {
                RepairOutcome::Accepted { accepted_rank, .. } if *accepted_rank > 1 => {
                    Some(RejectionContext {
                        collision_type: e.collision_type.clone(),
                        chosen_rank: Some(*accepted_rank),
                        summary: format!("Chose rank {} over Top-1", accepted_rank),
                    })
                }
                RepairOutcome::CustomResolution { manual_changes } => Some(RejectionContext {
                    collision_type: e.collision_type.clone(),
                    chosen_rank: None,
                    summary: format!("Custom resolution with {} changes", manual_changes),
                }),
                RepairOutcome::Abandoned => Some(RejectionContext {
                    collision_type: e.collision_type.clone(),
                    chosen_rank: None,
                    summary: "Operation abandoned".to_string(),
                }),
                _ => None,
            })
            .take(10)
            .collect();

        kb
    }

    /// Detect behavioral patterns from statistics.
    fn detect_patterns(&mut self) {
        for (collision_type, stats) in &self.collision_stats {
            if stats.occurrences < 5 {
                continue; // Not enough data
            }

            let top1_rate = stats.top1_accepted as f64 / stats.occurrences as f64;
            let custom_rate = stats.custom_resolutions as f64 / stats.occurrences as f64;
            let abandon_rate = stats.abandonments as f64 / stats.occurrences as f64;

            // Pattern: Top-1 suggestions are frequently rejected
            if top1_rate < 0.3 && stats.occurrences >= 10 {
                self.patterns.push(BehaviorPattern {
                    description: format!(
                        "Top-1 suggestions for '{}' are rarely accepted ({:.0}%)",
                        collision_type,
                        top1_rate * 100.0
                    ),
                    confidence: 1.0 - top1_rate,
                    supporting_events: stats.occurrences,
                    hint: format!(
                        "Consider presenting alternative suggestions first for '{}' collisions",
                        collision_type
                    ),
                });
            }

            // Pattern: Custom resolutions are common
            if custom_rate > 0.5 && stats.occurrences >= 10 {
                self.patterns.push(BehaviorPattern {
                    description: format!(
                        "Custom resolutions are preferred for '{}' ({:.0}%)",
                        collision_type,
                        custom_rate * 100.0
                    ),
                    confidence: custom_rate,
                    supporting_events: stats.occurrences,
                    hint: format!(
                        "The suggestion algorithm may not capture project-specific constraints for '{}'",
                        collision_type
                    ),
                });
            }

            // Pattern: High abandonment rate
            if abandon_rate > 0.3 && stats.occurrences >= 10 {
                self.patterns.push(BehaviorPattern {
                    description: format!(
                        "High abandonment rate for '{}' ({:.0}%)",
                        collision_type,
                        abandon_rate * 100.0
                    ),
                    confidence: abandon_rate,
                    supporting_events: stats.occurrences,
                    hint: format!(
                        "Consider asking for clarification before suggesting repairs for '{}'",
                        collision_type
                    ),
                });
            }
        }

        // Sort patterns by confidence
        self.patterns.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Generate context hints for the LLM based on the knowledge base.
    /// This is injected into the CLI output when a collision is detected.
    pub fn generate_context_hints(&self, collision_type: &str) -> Vec<String> {
        let mut hints = Vec::new();

        // Add relevant patterns
        for pattern in &self.patterns {
            if pattern.description.contains(collision_type) {
                hints.push(pattern.hint.clone());
            }
        }

        // Add recent rejection context
        let recent_for_type: Vec<_> = self
            .recent_rejections
            .iter()
            .filter(|r| r.collision_type == collision_type)
            .collect();

        if !recent_for_type.is_empty() {
            hints.push(format!(
                "Recent history: {} similar collisions were not resolved with Top-1",
                recent_for_type.len()
            ));
        }

        hints
    }
}

/// Extended collision error with RAG context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintCollisionErrorWithContext {
    /// The original collision error.
    #[serde(flatten)]
    pub error: crate::collision::ConstraintCollisionError,

    /// Context hints from local knowledge base (for LLM self-correction).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub context_hints: Vec<String>,

    /// Historical statistics for this collision type in this project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub historical_stats: Option<CollisionStatistics>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telemetry_serialization() {
        let event = RepairSuggestionEvent {
            session_hash: "abc123".to_string(),
            timestamp: "2026-05-10T15:00:00Z".to_string(),
            collision_type: "circular_reference".to_string(),
            suggestions_count: 3,
            outcome: RepairOutcome::Accepted {
                accepted_rank: 1,
                accepted_score: 10.5,
            },
            weights_config: WeightsSnapshot {
                deletion: 1000.0,
                relation_change: 100.0,
                constant_modification: 10.0,
                relative_error: 1.0,
            },
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("accepted_rank"));
        assert!(json.contains("circular_reference"));
    }
}
