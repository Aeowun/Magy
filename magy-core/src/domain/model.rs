// Copyright (C) 2026 Zachary Joubert
//
// This file is part of Magy.
//
// Magy is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// Magy is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with Magy. If not, see <https://www.gnu.org/licenses/>.

use crate::domain::agent::Task;
use crate::domain::project::ProjectContext;
use crate::domain::project::ProjectPlan;
use crate::domain::tool::{ToolRequest, ToolResult};
use crate::Error;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ApprovalStatus {
    Approved,
    Denied,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExecutionOutcome {
    Executed(ToolResult),
    Denied,
    AwaitingApproval,
    /// The model requested completion. The runtime still has to verify the
    /// project and persist the task transition before it can complete.
    CompletionRequested,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRecord {
    pub task_id: String,
    pub request: ToolRequest,
    pub approval_status: ApprovalStatus,
    pub outcome: ExecutionOutcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationResult {
    pub command: String,
    pub passed: bool,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepResult {
    pub model_response: ModelResponse,
    pub action_record: Option<ActionRecord>,
    pub verification: Option<VerificationResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Idle,
    Starting,
    AwaitingPlan,
    Planning,
    PlanValidation,
    ExecutingTask,
    AwaitingModel,
    AwaitingApproval,
    ExecutingTool,
    AwaitingVerification,
    Verifying,
    Recovering,
    Stalled,
    Completed,
    Failed,
    Cancelled,
}

impl Default for RunState {
    fn default() -> Self {
        Self::Idle
    }
}

impl RunState {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Starting
                | Self::AwaitingPlan
                | Self::Planning
                | Self::PlanValidation
                | Self::ExecutingTask
                | Self::AwaitingModel
                | Self::AwaitingApproval
                | Self::ExecutingTool
                | Self::AwaitingVerification
                | Self::Verifying
                | Self::Recovering
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Stalled | Self::Completed | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum FailureReason {
    Model(String),
    Parse(String),
    Tool(String),
    Context(String),
    Verification(String),
    Interrupted(String),
    Internal(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "details", rename_all = "snake_case")]
pub enum RunOutcome {
    Completed { completed_task_ids: Vec<String> },
    Stalled(FailureReason),
    Failed(FailureReason),
    Cancelled(CancellationReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationReason {
    Requested,
    Interrupted,
}

impl RunOutcome {
    pub fn terminal_state(&self) -> Option<RunState> {
        match self {
            Self::Completed { .. } => Some(RunState::Completed),
            Self::Stalled(_) => Some(RunState::Stalled),
            Self::Failed(_) => Some(RunState::Failed),
            Self::Cancelled(_) => Some(RunState::Cancelled),
        }
    }
}

/// Explicit events accepted by the authoritative run state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunTransition {
    Start,
    RequestPlan,
    BeginPlanning,
    ValidatePlan,
    CommitPlan,
    BeginTask(String),
    AwaitModel,
    ExecuteTool,
    AwaitApproval,
    RequestVerification,
    BeginVerification,
    Recover,
    Stall,
    Complete(Vec<String>),
    Fail(FailureReason),
    Cancel(CancellationReason),
}

/// The authoritative run state machine. Terminal states always carry a
/// matching outcome and timestamp. Active states never expose a terminal
/// outcome. A terminal transition can happen only once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStateMachine {
    state: RunState,
    outcome: Option<RunOutcome>,
    started_at_ms: Option<u64>,
    terminal_at_ms: Option<u64>,
    terminal_event_count: u8,
    recovery: RecoveryMetadata,
}

impl Default for RunStateMachine {
    fn default() -> Self {
        Self {
            state: RunState::Idle,
            outcome: None,
            started_at_ms: None,
            terminal_at_ms: None,
            terminal_event_count: 0,
            recovery: RecoveryMetadata::default(),
        }
    }
}

impl RunStateMachine {
    pub fn state(&self) -> &RunState {
        &self.state
    }

    pub fn outcome(&self) -> Option<&RunOutcome> {
        self.outcome.as_ref()
    }

    pub fn started_at_ms(&self) -> Option<u64> {
        self.started_at_ms
    }

    pub fn terminal_at_ms(&self) -> Option<u64> {
        self.terminal_at_ms
    }

    pub fn terminal_event_count(&self) -> u8 {
        self.terminal_event_count
    }

    pub fn recovery(&self) -> &RecoveryMetadata {
        &self.recovery
    }

    pub fn transition(&mut self, transition: RunTransition) -> Result<(), Error> {
        let next_state = match (&self.state, &transition) {
            (RunState::Idle, RunTransition::Start) => RunState::Starting,
            (RunState::Starting, RunTransition::RequestPlan) => RunState::AwaitingPlan,
            (RunState::Starting, RunTransition::BeginTask(_)) => RunState::ExecutingTask,
            (RunState::AwaitingPlan, RunTransition::BeginPlanning) => RunState::Planning,
            (RunState::Planning, RunTransition::AwaitModel) => RunState::AwaitingModel,
            (RunState::AwaitingModel, RunTransition::ValidatePlan) => RunState::PlanValidation,
            (RunState::PlanValidation, RunTransition::CommitPlan) => RunState::Planning, // Back to planning for next cycle
            (RunState::Planning, RunTransition::BeginTask(_)) => RunState::ExecutingTask,
            (RunState::ExecutingTask, RunTransition::AwaitModel) => RunState::AwaitingModel,
            (RunState::AwaitingModel, RunTransition::ExecuteTool) => RunState::ExecutingTool,
            (RunState::AwaitingModel, RunTransition::Stall) => RunState::Stalled,
            (RunState::AwaitingApproval, RunTransition::ExecuteTool) => RunState::ExecutingTool,
            (RunState::AwaitingApproval, RunTransition::AwaitModel) => RunState::AwaitingModel,
            (RunState::AwaitingApproval, RunTransition::Recover) => RunState::Recovering,
            (RunState::ExecutingTool, RunTransition::AwaitApproval) => RunState::AwaitingApproval,
            (RunState::ExecutingTool, RunTransition::RequestVerification) => RunState::AwaitingVerification,
            (RunState::AwaitingVerification, RunTransition::BeginVerification) => RunState::Verifying,
            (RunState::ExecutingTool, RunTransition::AwaitModel) => RunState::AwaitingModel,
            (RunState::ExecutingTool, RunTransition::Recover) => RunState::Recovering,
            (RunState::ExecutingTool, RunTransition::Stall) => RunState::Stalled,
            (RunState::Verifying, RunTransition::RequestPlan) => RunState::AwaitingPlan,
            (RunState::Verifying, RunTransition::Recover) => RunState::Recovering,
            (RunState::Recovering, RunTransition::AwaitModel) => RunState::AwaitingModel,
            (RunState::Recovering, RunTransition::Stall) => RunState::Stalled,
            (RunState::Stalled, RunTransition::Recover) => RunState::Recovering,
            (RunState::Stalled, RunTransition::AwaitModel) => RunState::AwaitingModel,
            (state, RunTransition::Complete(_)) if state.is_active() => RunState::Completed,
            (state, RunTransition::Stall) if state.is_active() => RunState::Stalled,
            (state, RunTransition::Fail(_)) if state.is_active() => RunState::Failed,
            (state, RunTransition::Cancel(_)) if state.is_active() => RunState::Cancelled,
            _ => return Err(Error::InvalidStateTransition),
        };

        if self.state == RunState::Idle && next_state == RunState::Starting {
            self.started_at_ms = Some(now_ms());
            self.terminal_at_ms = None;
            self.terminal_event_count = 0;
            self.outcome = None;
            self.recovery = RecoveryMetadata::default();
        }

        match transition {
            RunTransition::Complete(completed_task_ids) => {
                self.outcome = Some(RunOutcome::Completed { completed_task_ids });
            }
            RunTransition::Stall => {
                self.outcome = Some(RunOutcome::Stalled(
                    self.recovery
                        .last_failure
                        .clone()
                        .unwrap_or_else(|| FailureReason::Internal("Run stalled".to_string())),
                ));
            }
            RunTransition::Fail(reason) => {
                self.recovery.last_failure = Some(reason.clone());
                self.outcome = Some(RunOutcome::Failed(reason));
            }
            RunTransition::Cancel(reason) => {
                self.outcome = Some(RunOutcome::Cancelled(reason));
            }
            _ => {
                if !next_state.is_terminal() {
                    self.outcome = None;
                }
            }
        }

        self.state = next_state;
        if self.state.is_terminal() {
            self.terminal_at_ms = Some(now_ms());
            self.terminal_event_count = self.terminal_event_count.saturating_add(1);
            self.recovery.retryable = false;
        }
        Ok(())
    }

    pub fn start(&mut self, max_recovery_attempts: usize) {
        if self.state == RunState::Idle {
            let _ = self.transition(RunTransition::Start);
        }
        self.recovery.max_attempts = max_recovery_attempts;
        self.recovery.retryable = true;
    }

    pub fn begin_planning(&mut self) {
        let _ = self.transition(RunTransition::BeginPlanning);
    }

    pub fn request_plan(&mut self) {
        let _ = self.transition(RunTransition::RequestPlan);
    }

    pub fn validate_plan(&mut self) {
        let _ = self.transition(RunTransition::ValidatePlan);
    }

    pub fn commit_plan(&mut self) {
        let _ = self.transition(RunTransition::CommitPlan);
    }

    pub fn begin_task(&mut self, task_id: &str) {
        let _ = self.transition(RunTransition::BeginTask(task_id.to_string()));
    }

    pub fn await_model(&mut self) {
        let _ = self.transition(RunTransition::AwaitModel);
    }

    pub fn execute_tool(&mut self) {
        let _ = self.transition(RunTransition::ExecuteTool);
    }

    pub fn await_approval(&mut self) {
        let _ = self.transition(RunTransition::AwaitApproval);
    }

    pub fn request_verification(&mut self) {
        let _ = self.transition(RunTransition::RequestVerification);
    }

    pub fn begin_verification(&mut self) {
        let _ = self.transition(RunTransition::BeginVerification);
    }

    pub fn recover(&mut self) {
        let _ = self.transition(RunTransition::Recover);
    }

    pub fn stall(&mut self) {
        let _ = self.transition(RunTransition::Stall);
    }

    pub fn complete(&mut self, completed_task_ids: Vec<String>) {
        let _ = self.transition(RunTransition::Complete(completed_task_ids));
    }

    pub fn fail(&mut self, reason: FailureReason) {
        let _ = self.transition(RunTransition::Fail(reason));
    }

    pub fn cancel(&mut self, reason: CancellationReason) {
        let _ = self.transition(RunTransition::Cancel(reason));
    }

    pub fn record_recovery_attempt(&mut self, failure: FailureReason) -> bool {
        self.recovery.attempts = self.recovery.attempts.saturating_add(1);
        self.recovery.last_failure = Some(failure);
        self.recovery.attempts <= self.recovery.max_attempts
    }

    pub fn record_verification_failure(&mut self, attempts: usize, command: &str) {
        self.recovery.attempts = attempts;
        self.recovery.last_failure = Some(FailureReason::Verification(format!(
            "Verification command failed: {}",
            command
        )));
    }

    pub fn validate(&self) -> bool {
        match (&self.state, &self.outcome) {
            (state, None) if !state.is_terminal() => {
                self.terminal_at_ms.is_none() && self.terminal_event_count == 0
            }
            (RunState::Stalled, Some(RunOutcome::Stalled(_)))
            | (RunState::Completed, Some(RunOutcome::Completed { .. }))
            | (RunState::Failed, Some(RunOutcome::Failed(_)))
            | (RunState::Cancelled, Some(RunOutcome::Cancelled(_))) => {
                self.terminal_at_ms.is_some() && self.terminal_event_count == 1
            }
            _ => false,
        }
    }
}

pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryMetadata {
    pub attempts: usize,
    pub max_attempts: usize,
    pub retryable: bool,
    pub last_failure: Option<FailureReason>,
}

impl Default for RecoveryMetadata {
    fn default() -> Self {
        Self {
            attempts: 0,
            max_attempts: 0,
            retryable: false,
            last_failure: None,
        }
    }
}

/// The summary of a bounded execution cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub steps: Vec<StepResult>,
    #[serde(default)]
    pub run: RunStateMachine,
    /// Compatibility projection for older clients. New code must use
    /// `run.outcome` and `run.state`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stopped_reason: String,
}

impl ExecutionTrace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, max_recovery_attempts: usize) {
        self.run.start(max_recovery_attempts);
    }

    pub fn outcome(&self) -> Option<&RunOutcome> {
        self.run.outcome.as_ref()
    }

    pub fn validate(&self) -> bool {
        self.run.validate()
    }
}

impl Default for ExecutionTrace {
    fn default() -> Self {
        Self {
            steps: Vec::new(),
            run: RunStateMachine::default(),
            stopped_reason: String::new(),
        }
    }
}

/// The result of a coordinated agent run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub project_completed: bool,
    pub completed_task_ids: Vec<String>,
    pub active_task_id: Option<String>,
    pub state: RunState,
    pub outcome: Option<RunOutcome>,
    pub recovery: RecoveryMetadata,
    /// Compatibility projection for clients that still display a message.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stop_reason: String,
    pub agent: crate::domain::agent::Agent,
    pub project: crate::domain::project::Project,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannerRequest {
    pub system_prompt: String,
    pub goal: String,
    pub requirements: Vec<String>,
    pub constraints: Vec<String>,
    pub definition_of_done: Vec<String>,
    pub existing_tasks: Vec<crate::domain::project::ProjectTask>,
    pub context: ProjectContext,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannerResponse {
    pub plan_version: u32,
    pub tasks: Vec<crate::domain::project::ProjectTask>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutorRequest {
    pub system_prompt: String,
    pub task_id: String,
    pub description: String,
    pub acceptance_criteria: Vec<String>,
    pub allowed_scope: Vec<std::path::PathBuf>,
    pub local_context: ProjectContext,
    pub history: Vec<StepResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    pub system_prompt: String,
    pub context: ProjectContext,
    pub task: Option<Task>,
    pub plan: Option<ProjectPlan>,
    pub history: Vec<StepResult>,
    pub schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelResponse {
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelAction {
    ToolCall(ToolRequest),
}

pub trait ModelProvider {
    fn ask(&self, request: ModelRequest) -> Result<ModelResponse, Error>;
    fn plan(&self, request: PlannerRequest) -> Result<PlannerResponse, Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_outcomes_match_run_state() {
        let mut machine = RunStateMachine::default();
        assert!(machine.validate());
        machine.start(2);
        assert_eq!(machine.state(), &RunState::Starting);
        assert!(machine.validate());
        machine.fail(FailureReason::Parse("bad action".to_string()));
        assert_eq!(machine.state(), &RunState::Failed);
        assert!(machine.validate());
        assert!(!machine.state().is_active());
    }

    #[test]
    fn recovery_metadata_is_bounded() {
        let mut machine = RunStateMachine::default();
        machine.start(2);
        assert!(machine.record_recovery_attempt(FailureReason::Tool("first".to_string())));
        assert!(machine.record_recovery_attempt(FailureReason::Tool("second".to_string())));
        assert!(!machine.record_recovery_attempt(FailureReason::Tool("third".to_string())));
        assert_eq!(machine.recovery().attempts, 3);
        assert_eq!(machine.recovery().max_attempts, 2);
        assert!(machine.validate());
    }

    #[test]
    fn typed_transitions_cover_active_phases_and_terminal_once() {
        let mut machine = RunStateMachine::default();
        machine.transition(RunTransition::Start).unwrap();
        assert_eq!(machine.state(), &RunState::Starting);
        assert!(machine.started_at_ms().is_some());

        machine.transition(RunTransition::RequestPlan).unwrap();
        machine.transition(RunTransition::BeginPlanning).unwrap();
        machine.transition(RunTransition::AwaitModel).unwrap();
        machine.transition(RunTransition::ValidatePlan).unwrap();
        machine.transition(RunTransition::CommitPlan).unwrap();
        machine.transition(RunTransition::BeginTask("1".to_string())).unwrap();
        machine.transition(RunTransition::AwaitModel).unwrap();
        machine.transition(RunTransition::ExecuteTool).unwrap();
        machine.transition(RunTransition::RequestVerification).unwrap();
        machine
            .transition(RunTransition::BeginVerification)
            .unwrap();
        machine.transition(RunTransition::Complete(vec![])).unwrap();

        assert_eq!(machine.state(), &RunState::Completed);
        assert!(machine.state().is_terminal());
        assert!(!machine.state().is_active());
        assert!(machine.outcome().is_some());
        assert!(machine.terminal_at_ms().is_some());
        assert_eq!(machine.terminal_event_count(), 1);
        assert!(machine.validate());
        assert_eq!(
            machine.transition(RunTransition::Fail(FailureReason::Interrupted(
                "late cancellation".to_string()
            ))),
            Err(Error::InvalidStateTransition)
        );
        assert_eq!(machine.terminal_event_count(), 1);
    }

    #[test]
    fn awaiting_approval_is_active_without_terminal_outcome() {
        let mut machine = RunStateMachine::default();
        machine.transition(RunTransition::Start).unwrap();
        machine.transition(RunTransition::RequestPlan).unwrap();
        machine.transition(RunTransition::BeginPlanning).unwrap();
        machine.transition(RunTransition::BeginTask("1".to_string())).unwrap();
        machine.transition(RunTransition::AwaitModel).unwrap();
        machine.transition(RunTransition::ExecuteTool).unwrap();
        machine.transition(RunTransition::AwaitApproval).unwrap();

        assert_eq!(machine.state(), &RunState::AwaitingApproval);
        assert!(machine.state().is_active());
        assert!(!machine.state().is_terminal());
        assert!(machine.outcome().is_none());
        assert!(machine.terminal_at_ms().is_none());
        assert_eq!(machine.terminal_event_count(), 0);
        assert!(machine.validate());
    }

    #[test]
    fn cancellation_is_terminal_and_idempotent() {
        let mut machine = RunStateMachine::default();
        machine.transition(RunTransition::Start).unwrap();
        machine.transition(RunTransition::RequestPlan).unwrap();
        machine.transition(RunTransition::BeginPlanning).unwrap();
        machine.transition(RunTransition::AwaitModel).unwrap();
        machine
            .transition(RunTransition::Cancel(CancellationReason::Requested))
            .unwrap();

        assert_eq!(machine.state(), &RunState::Cancelled);
        assert!(matches!(
            machine.outcome(),
            Some(RunOutcome::Cancelled(CancellationReason::Requested))
        ));
        assert_eq!(machine.terminal_event_count(), 1);
        assert!(machine.validate());
        assert_eq!(
            machine.transition(RunTransition::Cancel(CancellationReason::Interrupted)),
            Err(Error::InvalidStateTransition)
        );
        assert_eq!(machine.terminal_event_count(), 1);
    }
}
