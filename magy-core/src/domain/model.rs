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

/// The summary of a bounded execution cycle.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub steps: Vec<StepResult>,
    pub stopped_reason: String,
}

impl ExecutionTrace {
    pub fn new() -> Self {
        Self::default()
    }
}

/// The result of a coordinated agent run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub project_completed: bool,
    pub completed_task_ids: Vec<String>,
    pub active_task_id: Option<String>,
    pub stop_reason: String,
}

#[derive(Debug, Clone)]
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
}
