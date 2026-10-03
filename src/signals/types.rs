//! Signal configuration and result types.
//! These types describe the on-disk signal schema; parts of it are written and
//! read by agents/hooks rather than by the binary's own code paths.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Different ways a node can transition to the next stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SignalConfig {
    /// Explicit human approval gate (plan→implement, merge approval).
    Breakpoint(BreakpointSignalConfig),
    /// Automatic detection via Stop hook + stop reason parsing (impl→test, test→merge).
    Hook(HookSignalConfig),
}

/// Explicit human approval gate — pauses pipeline until human signals proceed/reject.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakpointSignalConfig {
    /// Message shown in TUI and printed to the harness pane.
    pub message: String,
    /// Node id to reset and retry when the breakpoint is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_node: Option<String>,
}

/// Automatic state transition via Stop hook + stop reason routing.
/// The agent calls a tool to signal completion/error, or the hook detects via stop reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookSignalConfig {
    /// How to determine if the agent's work is complete (not just one message done).
    #[serde(default)]
    pub completion: CompletionCondition,

    /// Route different stop reasons to different outcomes.
    /// E.g., `turn_complete` → advance; `error` → retry with feedback; `user_input` → pause
    #[serde(default)]
    pub stop_reason_routes: HashMap<String, StopReasonRoute>,

    /// Default behavior if stop reason doesn't match any route.
    #[serde(default)]
    pub default_route: StopReasonRoute,
}

/// When the agent's work on this node is considered complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum CompletionCondition {
    /// Agent must explicitly call the signal tool to mark done.
    /// This is the safest: agent declares "I've finished implementation".
    #[serde(rename = "explicit")]
    Explicit {
        /// Optional: also look for this pattern in agent output as a fallback.
        /// E.g. "I've finished testing" or "Tests passing, ready for merge".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback_pattern: Option<String>,
    },
    /// Advance after this many turns with Stop hook fire, regardless of content.
    /// Risk: agent may not be done after N turns. Use with well-scoped prompts.
    #[serde(rename = "turn_threshold")]
    TurnThreshold {
        /// Min turns before eligible to advance.
        min_turns: usize,
    },
    /// Advance after N consecutive turns with no code changes (idle threshold).
    /// Agent is assumed done when output stabilizes.
    #[serde(rename = "idle_threshold")]
    IdleThreshold {
        /// Turns with no code changes before advancing.
        idle_turns: usize,
    },
}

impl Default for CompletionCondition {
    fn default() -> Self {
        CompletionCondition::Explicit {
            fallback_pattern: None,
        }
    }
}

/// How to handle a specific stop reason from the agent's Stop hook.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StopReasonRoute {
    /// Advance to next node in the pipeline.
    #[default]
    Next,
    /// Fail the pipeline with this stop reason as the error.
    Fail,
    /// Reset this node to Pending and retry, optionally injecting the stop reason as feedback.
    Retry,
    /// Pause the node; wait for human intervention (e.g., user_input from permission prompt).
    Pause,
}

/// Outcome of polling a signal. May come from breakpoint, hook, or agent tool.
#[derive(Debug, Clone)]
pub struct SignalResult {
    /// Whether the signal approved transition to next stage (true) or rejected (false).
    pub approved: bool,
    /// Feedback to inject into retry_node's next prompt (if node will be retried).
    pub feedback: Option<String>,
    /// The stop reason that triggered this signal (if from Stop hook or agent tool).
    pub stop_reason: Option<String>,
}

impl SignalResult {
    /// Create an approval result (advance to next stage).
    pub fn approved(stop_reason: Option<String>) -> Self {
        SignalResult {
            approved: true,
            feedback: None,
            stop_reason,
        }
    }

    /// Create a rejection result (retry with feedback).
    pub fn rejected(feedback: String, stop_reason: Option<String>) -> Self {
        SignalResult {
            approved: false,
            feedback: Some(feedback),
            stop_reason,
        }
    }
}

/// Stop reason emitted by agent hooks or explicitly by agent tool calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Agent's turn completed (Stop hook fired).
    TurnComplete,
    /// Agent encountered an error and stopped.
    Error,
    /// Agent is blocked on user input (AskUserQuestion / permission prompt).
    UserInput,
    /// Agent explicitly marked this node as done.
    ExplicitDone,
    /// Custom reason emitted by agent.
    #[serde(untagged)]
    Custom(String),
}

impl StopReason {
    pub fn as_str(&self) -> &str {
        match self {
            StopReason::TurnComplete => "turn_complete",
            StopReason::Error => "error",
            StopReason::UserInput => "user_input",
            StopReason::ExplicitDone => "explicit_done",
            StopReason::Custom(s) => s,
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "turn_complete" => StopReason::TurnComplete,
            "error" => StopReason::Error,
            "user_input" => StopReason::UserInput,
            "explicit_done" => StopReason::ExplicitDone,
            other => StopReason::Custom(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_result_approved() {
        let result = SignalResult::approved(Some("turn_complete".to_string()));
        assert!(result.approved);
        assert_eq!(result.feedback, None);
        assert_eq!(result.stop_reason, Some("turn_complete".to_string()));
    }

    #[test]
    fn test_signal_result_rejected() {
        let result = SignalResult::rejected("test failed".to_string(), Some("error".to_string()));
        assert!(!result.approved);
        assert_eq!(result.feedback, Some("test failed".to_string()));
        assert_eq!(result.stop_reason, Some("error".to_string()));
    }

    #[test]
    fn test_stop_reason_conversion() {
        assert_eq!(StopReason::TurnComplete.as_str(), "turn_complete");
        assert_eq!(
            StopReason::from_str("turn_complete"),
            StopReason::TurnComplete
        );
        assert_eq!(StopReason::Error.as_str(), "error");
        assert_eq!(StopReason::from_str("error"), StopReason::Error);
    }

    #[test]
    fn test_completion_condition_default() {
        let cc = CompletionCondition::default();
        match cc {
            CompletionCondition::Explicit {
                fallback_pattern: None,
            } => (),
            _ => panic!("Expected explicit completion by default"),
        }
    }

    #[test]
    fn test_stop_reason_route_default() {
        let route = StopReasonRoute::default();
        assert_eq!(route, StopReasonRoute::Next);
    }

    #[test]
    fn test_signal_config_breakpoint_serialization() {
        let config = SignalConfig::Breakpoint(BreakpointSignalConfig {
            message: "Test message".to_string(),
            retry_node: Some("retry".to_string()),
        });

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: SignalConfig = serde_json::from_str(&json).unwrap();

        match deserialized {
            SignalConfig::Breakpoint(cfg) => {
                assert_eq!(cfg.message, "Test message");
                assert_eq!(cfg.retry_node, Some("retry".to_string()));
            }
            _ => panic!("Expected breakpoint config"),
        }
    }
}
