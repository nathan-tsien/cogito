//! Static catalog of chaos test scenarios. Each scenario is a fully
//! deterministic recipe (one `user_input` + an ordered sequence of model
//! event scripts, one per model call the turn will make) used by the
//! `resume_chaos` test main.
//!
//! Pure data — no traits, no async, no I/O. Determinism is enforced by
//! the static `Vec` shapes plus `ScriptedMockModel`'s deterministic
//! matcher dispatch (P5.2).

use cogito_protocol::content::ContentBlock;
use cogito_protocol::gateway::{ModelEvent, StopReason, Usage};

/// One end-to-end chaos test recipe.
#[derive(Debug, Clone)]
pub struct ChaosScenario {
    /// Stable identifier used in test failure messages.
    pub name: &'static str,
    /// User input for the single turn the scenario drives.
    pub user_input: Vec<ContentBlock>,
    /// Ordered scripts for the model calls the turn will make.
    /// `model_scripts[i]` is consumed by the i-th `ModelGateway::stream` call.
    /// Determinism: static vectors + cloned per call means the same input
    /// produces the same stream across pre-crash and post-resume executions.
    pub model_scripts: Vec<Vec<ModelEvent>>,
    /// Whether this scenario exercises the `PausedOnJob` flow.
    /// v0.1 actor returns `JobManagerUnavailable` for the paused-job
    /// `ResumePoint` variants, so scenarios with `uses_async_job = true`
    /// are skipped by P5.6's chaos main in v0.1. The data is still
    /// present so Sprint 4 can enable it without re-authoring scenarios.
    pub uses_async_job: bool,
}

/// All registered scenarios, in canonical order.
#[must_use]
pub fn all() -> Vec<ChaosScenario> {
    vec![
        single_tool_happy_path(),
        no_tool_short_turn(),
        tool_returns_error(),
        paused_async_job(),
        thinking_then_text_then_tool(),
        text_then_skill_then_tool(),
        plugin_skill_then_tool(),
    ]
}

/// Scenario 1: model emits a single tool call, tool succeeds, model
/// emits final assistant text on the second call, `EndTurn`.
#[must_use]
pub fn single_tool_happy_path() -> ChaosScenario {
    ChaosScenario {
        name: "single_tool_happy_path",
        user_input: vec![ContentBlock::Text {
            text: "read /etc/hostname".into(),
        }],
        model_scripts: vec![
            // Call 1: announce + complete a single tool_use, stop_reason=tool_use
            vec![
                ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "Reading file...".into(),
                },
                ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "Reading file...".into(),
                },
                ModelEvent::ToolUseStarted {
                    block_index: 1,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                },
                ModelEvent::ToolUseCompleted {
                    block_index: 1,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                    args: serde_json::json!({"path": "/etc/hostname"}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage {
                        input_tokens: 50,
                        output_tokens: 20,
                    },
                },
            ],
            // Call 2: assistant emits the final reply, EndTurn.
            vec![
                ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "The hostname is foo.".into(),
                },
                ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "The hostname is foo.".into(),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 75,
                        output_tokens: 10,
                    },
                },
            ],
        ],
        uses_async_job: false,
    }
}

/// Scenario 2: no tools, one short assistant reply, `EndTurn`.
#[must_use]
pub fn no_tool_short_turn() -> ChaosScenario {
    ChaosScenario {
        name: "no_tool_short_turn",
        user_input: vec![ContentBlock::Text {
            text: "say hi".into(),
        }],
        model_scripts: vec![vec![
            ModelEvent::TextDelta {
                block_index: 0,
                chunk: "Hi.".into(),
            },
            ModelEvent::TextBlockCompleted {
                block_index: 0,
                text: "Hi.".into(),
            },
            ModelEvent::MessageCompleted {
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 5,
                    output_tokens: 2,
                },
            },
        ]],
        uses_async_job: false,
    }
}

/// Scenario 3: tool dispatch is shaped like #1, but the test harness
/// wires the tool to return `ToolResult::Error` so the second model call
/// produces an explanatory final reply.
#[must_use]
pub fn tool_returns_error() -> ChaosScenario {
    ChaosScenario {
        name: "tool_returns_error",
        user_input: vec![ContentBlock::Text {
            text: "read /nonexistent".into(),
        }],
        model_scripts: vec![
            vec![
                ModelEvent::ToolUseStarted {
                    block_index: 0,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                },
                ModelEvent::ToolUseCompleted {
                    block_index: 0,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                    args: serde_json::json!({"path": "/nonexistent"}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
            ],
            vec![
                ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "File not found.".into(),
                },
                ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "File not found.".into(),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                },
            ],
        ],
        uses_async_job: false,
    }
}

/// Scenario 4: model emits one async-tool call; the tool submits a job
/// and the actor pauses on the job. Sprint 4 will exercise the resume
/// path; v0.1 P5.6 skips this scenario.
#[must_use]
pub fn paused_async_job() -> ChaosScenario {
    ChaosScenario {
        name: "paused_async_job",
        user_input: vec![ContentBlock::Text {
            text: "run long task".into(),
        }],
        model_scripts: vec![
            vec![
                ModelEvent::ToolUseStarted {
                    block_index: 0,
                    call_id: "c_async".into(),
                    tool_name: "long_tool".into(),
                },
                ModelEvent::ToolUseCompleted {
                    block_index: 0,
                    call_id: "c_async".into(),
                    tool_name: "long_tool".into(),
                    args: serde_json::json!({}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage::default(),
                },
            ],
            vec![
                ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "Done.".into(),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                },
            ],
        ],
        uses_async_job: true,
    }
}

/// Scenario 5: assistant turn starts with a thinking block (carrying
/// `provider_opaque` signature), then text, then a tool call. Verifies
/// that H03 resume preserves `ThinkingBlockRecorded` events in the
/// prefix and that H04's projection rebuilds the assistant
/// `Message::content` with `Thinking` at index 0. Added for ADR-0019.
#[must_use]
pub fn thinking_then_text_then_tool() -> ChaosScenario {
    ChaosScenario {
        name: "thinking_then_text_then_tool",
        user_input: vec![ContentBlock::Text {
            text: "read /etc/hostname".into(),
        }],
        model_scripts: vec![
            vec![
                ModelEvent::ThinkingDelta {
                    block_index: 0,
                    chunk: "I should ".into(),
                },
                ModelEvent::ThinkingDelta {
                    block_index: 0,
                    chunk: "read the file.".into(),
                },
                ModelEvent::ThinkingBlockCompleted {
                    block_index: 0,
                    text: "I should read the file.".into(),
                    provider_opaque: Some(serde_json::json!({"signature": "sig_abc"})),
                },
                ModelEvent::TextDelta {
                    block_index: 1,
                    chunk: "Reading file...".into(),
                },
                ModelEvent::TextBlockCompleted {
                    block_index: 1,
                    text: "Reading file...".into(),
                },
                ModelEvent::ToolUseStarted {
                    block_index: 2,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                },
                ModelEvent::ToolUseCompleted {
                    block_index: 2,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                    args: serde_json::json!({"path": "/etc/hostname"}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage {
                        input_tokens: 50,
                        output_tokens: 25,
                    },
                },
            ],
            vec![
                ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "The hostname is foo.".into(),
                },
                ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "The hostname is foo.".into(),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 75,
                        output_tokens: 10,
                    },
                },
            ],
        ],
        uses_async_job: false,
    }
}

/// Scenario 6: assistant turn 1 emits text containing a model-channel sigil
/// `$foo`, then a tool call; the tool returns; the assistant emits a final
/// reply. A separate turn 2 (driven by the chaos runner) then re-derives
/// activation of `foo` from turn 1's text. Added for ADR-0020 / Sprint 7.
///
/// The chaos runner that drives this scenario lives inline in
/// `cogito-core/tests/resume_chaos.rs` (it needs a `SkillProvider` and the
/// `SystemPromptInjectorConfig::Skill` strategy, which `cogito-test-fixtures`
/// cannot wire without crossing the `cogito-protocol`-only dep boundary).
///
/// The runner uses this scenario's `model_scripts[0]` for turn 1 call 1
/// (text-then-tool) and `model_scripts[1]` for turn 1 call 2 (after the
/// tool result). Turn 2's single model call is scripted inline in the test.
#[must_use]
pub fn text_then_skill_then_tool() -> ChaosScenario {
    ChaosScenario {
        name: "text_then_skill_then_tool",
        user_input: vec![ContentBlock::Text {
            text: "please use $foo".into(),
        }],
        model_scripts: vec![
            // Call 1: assistant emits a sigil-containing text block, then a
            // tool_use, stop_reason=tool_use.
            vec![
                ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "Sure, $foo please. ".into(),
                },
                ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "Sure, $foo please. ".into(),
                },
                ModelEvent::ToolUseStarted {
                    block_index: 1,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                },
                ModelEvent::ToolUseCompleted {
                    block_index: 1,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                    args: serde_json::json!({"path": "/etc/hostname"}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage {
                        input_tokens: 50,
                        output_tokens: 20,
                    },
                },
            ],
            // Call 2 (post-tool): assistant emits the final turn 1 reply,
            // EndTurn.
            vec![
                ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "Done reading.".into(),
                },
                ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "Done reading.".into(),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 75,
                        output_tokens: 10,
                    },
                },
            ],
        ],
        uses_async_job: false,
    }
}

/// Scenario 7: identical control flow to `text_then_skill_then_tool`, but the
/// activated skill is a **plugin-loaded** skill addressed by its namespaced
/// name `$acme:review` (ADR-0021 §3: plugin skills register as
/// `<plugin_id>:<name>`; the sigil regex admits `:`). The chaos runner wires a
/// `SkillProvider` whose single skill carries `SkillSource::Plugin`, so a crash
/// while that plugin skill is mid-activation exercises the same H06 sigil /
/// H11 injection idempotency path with a Plugin-scoped source. Added for
/// Sprint 13 (v0.2 hardening).
///
/// As with scenario 6, the runner lives inline in
/// `cogito-core/tests/resume_chaos.rs`: `model_scripts[0]` drives turn 1 call 1
/// (sigil text + tool use), `model_scripts[1]` drives turn 1 call 2 (after the
/// tool result), and turn 2's single model call is scripted inline.
#[must_use]
pub fn plugin_skill_then_tool() -> ChaosScenario {
    ChaosScenario {
        name: "plugin_skill_then_tool",
        user_input: vec![ContentBlock::Text {
            text: "please use $acme:review".into(),
        }],
        model_scripts: vec![
            // Call 1: assistant emits a namespaced-sigil text block, then a
            // tool_use, stop_reason=tool_use.
            vec![
                ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "Sure, $acme:review please. ".into(),
                },
                ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "Sure, $acme:review please. ".into(),
                },
                ModelEvent::ToolUseStarted {
                    block_index: 1,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                },
                ModelEvent::ToolUseCompleted {
                    block_index: 1,
                    call_id: "c1".into(),
                    tool_name: "read_file".into(),
                    args: serde_json::json!({"path": "/etc/hostname"}),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::ToolUse,
                    usage: Usage {
                        input_tokens: 50,
                        output_tokens: 20,
                    },
                },
            ],
            // Call 2 (post-tool): assistant emits the final turn 1 reply,
            // EndTurn.
            vec![
                ModelEvent::TextDelta {
                    block_index: 0,
                    chunk: "Done reading.".into(),
                },
                ModelEvent::TextBlockCompleted {
                    block_index: 0,
                    text: "Done reading.".into(),
                },
                ModelEvent::MessageCompleted {
                    stop_reason: StopReason::EndTurn,
                    usage: Usage {
                        input_tokens: 75,
                        output_tokens: 10,
                    },
                },
            ],
        ],
        uses_async_job: false,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn all_scenarios_have_unique_names() {
        let scenarios = all();
        let mut names: Vec<&'static str> = scenarios.iter().map(|s| s.name).collect();
        names.sort_unstable();
        let original_len = names.len();
        names.dedup();
        assert_eq!(names.len(), original_len, "duplicate scenario names");
    }

    #[test]
    fn every_scenario_has_at_least_one_script() {
        for s in all() {
            assert!(
                !s.model_scripts.is_empty(),
                "scenario {} has no model scripts",
                s.name
            );
        }
    }

    #[test]
    fn every_script_ends_with_message_completed() {
        for s in all() {
            for (i, script) in s.model_scripts.iter().enumerate() {
                let last = script.last().expect("script not empty");
                assert!(
                    matches!(last, ModelEvent::MessageCompleted { .. }),
                    "scenario {} script {i} does not end with MessageCompleted",
                    s.name
                );
            }
        }
    }
}
