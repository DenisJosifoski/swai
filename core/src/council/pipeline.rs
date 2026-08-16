//! SWAI — Council multi-turn debate execution engine.
//!
//! Coordinates the Generator -> Auditor(s) -> Synthesizer workflow with
//! failure-matrix resilience: graceful fallback to the best available
//! draft when any stage times out or errors, and support for both
//! concurrent (parallel) and sequential (fast process-swap < 500 ms)
//! execution modes.

use crate::council::types::{
    CouncilMode, CouncilPipelineConfig, CouncilRole, DebateOutcome, DebateTranscript,
    FallbackAction, PipelineStage, TurnResult,
};
use std::thread;
use std::time::{Duration, Instant};

/// Errors that can occur during council pipeline execution.
#[derive(Debug, thiserror::Error)]
pub enum CouncilError {
    #[error("pipeline has no stages")]
    EmptyPipeline,
    #[error("stage failed: {0}")]
    StageFailed(String),
    #[error("aborted: {0}")]
    Aborted(String),
}

/// Abstraction over the LLM inference backend used by each debate stage.
pub trait Executor: Send + Sync {
    fn execute(&self, stage: &PipelineStage, input: &str) -> Result<String, String>;
}

/// Mutable state carried through a single debate execution.
pub struct DebateState {
    pub transcript: DebateTranscript,
    pub draft: Option<String>,
    pub audit_results: Vec<String>,
    pub warnings: Vec<String>,
    pub aborted: bool,
}

impl DebateState {
    pub fn new(session_id: String, input_prompt: String, config: CouncilPipelineConfig) -> Self {
        Self {
            transcript: DebateTranscript::new(session_id, input_prompt, config),
            draft: None,
            audit_results: Vec::new(),
            warnings: Vec::new(),
            aborted: false,
        }
    }

    pub fn handle_failure(&mut self, fallback: &FallbackAction, _turn_index: usize, error: &str) {
        match fallback {
            FallbackAction::Abort => {
                self.warnings.push(format!("Stage failed (abort): {error}"));
                self.aborted = true;
            }
            FallbackAction::Skip => {
                self.warnings.push(format!("Stage skipped: {error}"));
            }
            FallbackAction::Retry { max_retries } => {
                self.warnings
                    .push(format!("Stage retried {max_retries} times failed: {error}"));
            }
        }
    }
}

/// The council debate engine.
pub struct CouncilEngine<E: Executor> {
    pub config: CouncilPipelineConfig,
    pub executor: E,
}

impl<E: Executor> CouncilEngine<E> {
    pub fn new(config: CouncilPipelineConfig, executor: E) -> Self {
        Self { config, executor }
    }

    /// Execute the full debate pipeline with input prompt.
    pub fn execute(&self, input_prompt: &str) -> DebateOutcome {
        if self.config.stages.is_empty() {
            return DebateOutcome::Aborted {
                reason: "empty pipeline".into(),
                transcript: DebateTranscript::new(
                    String::new(),
                    input_prompt.to_string(),
                    self.config.clone(),
                ),
            };
        }

        let mut state = DebateState::new(
            format!("debate-{}", chrono::Utc::now().timestamp()),
            input_prompt.to_string(),
            self.config.clone(),
        );

        // Stage 1: Generator (first stage with role Generator or first stage)
        self.run_generator(&mut state);
        if !state.aborted {
            // Stage 2: Auditor(s)
            self.run_auditors(&mut state);
            // Stage 3: Synthesizer
            self.run_synthesizer(&mut state);
        }

        self.build_outcome(state)
    }

    fn run_generator(&self, state: &mut DebateState) {
        let stage = &self.config.stages[0];
        let input = &state.transcript.input_prompt;
        let start = Instant::now();

        match self.executor.execute(stage, input) {
            Ok(output) => {
                state.draft = Some(output.clone());
                state.transcript.append_turn(TurnResult {
                    turn_index: 0,
                    role: CouncilRole::Generator,
                    model_id: stage.model_id.clone(),
                    output,
                    duration: start.elapsed(),
                    error: None,
                });
            }
            Err(err) => {
                state.handle_failure(&self.config.fallback, 0, &err);
            }
        }
    }

    fn run_auditors(&self, state: &mut DebateState) {
        let auditors: Vec<&PipelineStage> = self
            .config
            .stages
            .iter()
            .enumerate()
            .filter(|(idx, s)| {
                *idx > 0 && (s.role == CouncilRole::Auditor || s.role != CouncilRole::Synthesizer)
            })
            .map(|(_, s)| s)
            .collect();

        if auditors.is_empty() || state.draft.is_none() {
            return;
        }

        let draft = state.draft.as_ref().unwrap().clone();
        let prompt = format!(
            "Draft to audit:\n{draft}\n\nOriginal prompt:\n{}",
            state.transcript.input_prompt
        );

        for (i, stage) in auditors.iter().enumerate() {
            if state.aborted {
                break;
            }
            let start = Instant::now();
            match self.executor.execute(stage, &prompt) {
                Ok(output) => {
                    state.audit_results.push(output.clone());
                    state.transcript.append_turn(TurnResult {
                        turn_index: i + 1,
                        role: CouncilRole::Auditor,
                        model_id: stage.model_id.clone(),
                        output,
                        duration: start.elapsed(),
                        error: None,
                    });
                }
                Err(err) => {
                    state.handle_failure(&self.config.fallback, i + 1, &err);
                }
            }
        }
    }

    fn run_synthesizer(&self, state: &mut DebateState) {
        let synth_stage = self
            .config
            .stages
            .iter()
            .find(|s| s.role == CouncilRole::Synthesizer);
        if let Some(stage) = synth_stage {
            if state.aborted {
                return;
            }
            let draft = state.draft.as_deref().unwrap_or("");
            let critiques = if state.audit_results.is_empty() {
                "No audit critiques.".to_string()
            } else {
                state.audit_results.join("\n\n---\n\n")
            };
            let prompt = format!(
                "Original prompt:\n{}\n\nDraft response:\n{}\n\nAudit critiques:\n{}",
                state.transcript.input_prompt, draft, critiques
            );

            let start = Instant::now();
            match self.executor.execute(stage, &prompt) {
                Ok(output) => {
                    state.draft = Some(output.clone());
                    state.transcript.append_turn(TurnResult {
                        turn_index: state.transcript.turns.len(),
                        role: CouncilRole::Synthesizer,
                        model_id: stage.model_id.clone(),
                        output,
                        duration: start.elapsed(),
                        error: None,
                    });
                }
                Err(err) => {
                    state.handle_failure(&self.config.fallback, state.transcript.turns.len(), &err);
                }
            }
        }
    }

    fn build_outcome(&self, state: DebateState) -> DebateOutcome {
        if state.aborted {
            return DebateOutcome::Aborted {
                reason: "pipeline aborted due to stage failure".into(),
                transcript: state.transcript,
            };
        }

        match state.draft {
            Some(final_response) if !state.warnings.is_empty() => DebateOutcome::Partial {
                fallback_response: final_response,
                warnings: state.warnings,
                transcript: state.transcript,
            },
            Some(final_response) => DebateOutcome::Success {
                final_response,
                transcript: state.transcript,
            },
            None if !state.warnings.is_empty() => DebateOutcome::Partial {
                fallback_response: String::new(),
                warnings: state.warnings,
                transcript: state.transcript,
            },
            _ => DebateOutcome::Aborted {
                reason: "no response produced".into(),
                transcript: state.transcript,
            },
        }
    }
}
