//! Automatic state transition via Stop hook + stop reason routing.
//! Hook-driven signalling is consumed through the signal files, not from the
//! binary's own call graph.
#![allow(dead_code)]

use super::{CompletionCondition, HookSignalConfig, Signal, SignalResult, StopReasonRoute, paths};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
struct HookSignalFile {
    stop_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    feedback: Option<String>,
    #[serde(default)]
    turn_count: usize,
    #[serde(default)]
    idle_turns: usize,
}

/// Automatic hook-based signal: detects completion via Stop hook + stop reason routing.
/// Optionally requires explicit agent marker or completion thresholds before advancing.
pub struct HookSignal {
    node_id: String,
    task_id: String,
    pane: Option<String>,
    config: HookSignalConfig,
}

impl HookSignal {
    pub fn new(node_id: &str, task_id: &str, pane: Option<&str>, config: HookSignalConfig) -> Self {
        HookSignal {
            node_id: node_id.to_string(),
            task_id: task_id.to_string(),
            pane: pane.map(|s| s.to_string()),
            config,
        }
    }

    /// Check for explicit agent signal (tool call or fallback pattern).
    fn check_explicit_signal(&self) -> Result<Option<HookSignalFile>> {
        let path = paths::hook_signal(&self.task_id, &self.node_id);
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let signal: HookSignalFile = serde_json::from_str(&content)?;
            return Ok(Some(signal));
        }
        Ok(None)
    }

    /// Check if Stop hook has fired (turn completed).
    fn check_turn_done(&self) -> Result<bool> {
        let Some(pane) = &self.pane else {
            return Ok(false);
        };
        let path = paths::turn_done_marker(pane);
        Ok(path.exists())
    }

    /// Evaluate completion condition. Returns true if work is considered complete.
    fn is_complete(&self, _signal: &HookSignalFile, _turn_count: usize) -> Result<bool> {
        match &self.config.completion {
            CompletionCondition::Explicit {
                fallback_pattern: _,
            } => {
                // Explicit completion requires the agent to have called the signal tool.
                // If we reach here, the signal was found, so we're done.
                Ok(true)
            }
            CompletionCondition::TurnThreshold { min_turns } => {
                // Simple: just need N turns to complete.
                Ok(_turn_count >= *min_turns)
            }
            CompletionCondition::IdleThreshold { idle_turns } => {
                // Wait for idle_turns with no code changes.
                // For now, use the idle_turns count from the signal file.
                Ok(_signal.idle_turns >= *idle_turns)
            }
        }
    }

    /// Route the stop reason to a decision.
    fn route_stop_reason(&self, reason: &str) -> StopReasonRoute {
        self.config
            .stop_reason_routes
            .get(reason)
            .cloned()
            .unwrap_or(self.config.default_route.clone())
    }
}

impl Signal for HookSignal {
    fn check(&self) -> Result<Option<SignalResult>> {
        // Check for explicit agent signal (via tool call).
        if let Some(signal) = self.check_explicit_signal()?
            && self.is_complete(&signal, 0)?
        {
            let route = self.route_stop_reason(&signal.stop_reason);
            match route {
                StopReasonRoute::Next => {
                    return Ok(Some(SignalResult::approved(Some(signal.stop_reason))));
                }
                StopReasonRoute::Retry => {
                    return Ok(Some(SignalResult::rejected(
                        signal.feedback.unwrap_or_else(|| {
                            format!(
                                "Agent signaled stop reason '{}', retrying",
                                signal.stop_reason
                            )
                        }),
                        Some(signal.stop_reason),
                    )));
                }
                StopReasonRoute::Fail => {
                    return Err(anyhow!(
                        "Stop reason '{}' routed to Fail",
                        signal.stop_reason
                    ));
                }
                StopReasonRoute::Pause => {
                    // Don't return yet; keep waiting for explicit done signal
                    return Ok(None);
                }
            }
        }

        // Check if Stop hook has fired (no explicit signal yet, but turn is done).
        // For now, if there's no explicit signal but completion is Explicit mode,
        // we keep waiting. If it's threshold-based, we check the threshold.
        match &self.config.completion {
            CompletionCondition::Explicit { .. } => {
                // Waiting for explicit agent signal. Stop hook alone isn't enough.
                Ok(None)
            }
            CompletionCondition::TurnThreshold { min_turns } => {
                // Would need to track turn count elsewhere (in runner state).
                // For now, just check if turn is done at all.
                if self.check_turn_done()? && *min_turns == 1 {
                    let route = self.route_stop_reason("turn_complete");
                    match route {
                        StopReasonRoute::Next => Ok(Some(SignalResult::approved(Some(
                            "turn_complete".to_string(),
                        )))),
                        StopReasonRoute::Retry => Ok(Some(SignalResult::rejected(
                            "Turn completed, retrying".to_string(),
                            Some("turn_complete".to_string()),
                        ))),
                        StopReasonRoute::Fail => Err(anyhow!("Turn complete routed to Fail")),
                        StopReasonRoute::Pause => Ok(None),
                    }
                } else {
                    Ok(None)
                }
            }
            CompletionCondition::IdleThreshold { .. } => {
                // Similar: would need runner to track idle turns.
                Ok(None)
            }
        }
    }

    fn clear(&self) -> Result<()> {
        let path = paths::hook_signal(&self.task_id, &self.node_id);
        let _ = std::fs::remove_file(&path);

        if let Some(pane) = &self.pane {
            let marker_path = paths::turn_done_marker(pane);
            let _ = std::fs::remove_file(&marker_path);
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "hook"
    }
}
