//! Auto Drive diagnostics interposer.
//!
//! This crate will orchestrate a post-success verification pass whenever
//! Auto Drive reports `AutoCoordinatorStatus::Success`. It will force a
//! structured JSON response from the model indicating whether the
//! original goal is genuinely complete before allowing the run to exit.

#![allow(dead_code)]

use anyhow::Result;

/// Schema for the forced JSON diagnostics reply.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct CompletionCheck {
    pub complete: bool,
    pub explanation: String,
}

/// Configuration for diagnostics behaviour.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct DiagnosticsConfig {
    pub max_retries: u8,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self { max_retries: 2 }
    }
}

/// Placeholder diagnostics facade.
pub struct AutoDriveDiagnostics;

impl Default for AutoDriveDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoDriveDiagnostics {
    pub fn new() -> Self {
        Self
    }

    pub fn completion_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["complete", "explanation"],
            "properties": {
                "complete": { "type": "boolean" },
                "explanation": { "type": "string" }
            },
            "additionalProperties": false
        })
    }

    pub async fn run_check(&self, _goal: &str) -> Result<CompletionCheck> {
        unimplemented!("Diagnostics check not yet implemented");
    }
}
