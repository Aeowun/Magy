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

use crate::Error;
use crate::domain::agent::Agent;
use crate::domain::tool::ToolRequest;
use crate::domain::model::{ApprovalStatus, ExecutionTrace, ExecutionOutcome};
use crate::application::tool_execution::execute_tool;

/// A policy for evaluating whether a tool request requires approval.
pub trait ApprovalPolicy {
    fn evaluate(&self, request: &ToolRequest) -> ApprovalStatus;
}

/// A simple deterministic approval policy.
pub struct DefaultApprovalPolicy;

impl ApprovalPolicy for DefaultApprovalPolicy {
    fn evaluate(&self, request: &ToolRequest) -> ApprovalStatus {
        match request {
            ToolRequest::ReadFile { .. } => ApprovalStatus::Approved,
            ToolRequest::ListDirectory { .. } => ApprovalStatus::Approved,
            ToolRequest::DiscoverFiles => ApprovalStatus::Approved,
            ToolRequest::WriteFile { .. } => ApprovalStatus::Pending,
            ToolRequest::RunCommand { .. } => ApprovalStatus::Pending,
            ToolRequest::TaskComplete => ApprovalStatus::Approved,
        }
    }
}

/// Resolves a pending action in the execution trace.
///
/// Executing an approved action uses the exact request recorded in the trace.
pub fn resolve_pending_action(
    agent: &Agent,
    trace: &mut ExecutionTrace,
    step_index: usize,
    approved: bool,
) -> Result<(), Error> {
    let step = trace.steps.get_mut(step_index).ok_or(Error::ActionNotFound)?;
    let record = step.action_record.as_mut().ok_or(Error::ActionNotFound)?;

    if record.approval_status != ApprovalStatus::Pending {
        return Err(Error::ActionNotPending);
    }

    if approved {
        record.approval_status = ApprovalStatus::Approved;
        let res = execute_tool(agent, record.request.clone());
        record.outcome = ExecutionOutcome::Executed(res);
    } else {
        record.approval_status = ApprovalStatus::Denied;
        record.outcome = ExecutionOutcome::Denied;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::fs;
    use tempfile::tempdir;
    use crate::domain::model::{ActionRecord, StepResult, ModelResponse};
    use crate::domain::tool::ToolResult;
    use crate::application::project_lifecycle::open_project;
    use crate::application::task_lifecycle::select_task;

    #[test]
    fn test_resolve_pending_action_approve() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let mut trace = ExecutionTrace::new();
        trace.steps.push(StepResult {
            model_response: ModelResponse { content: "Writing".to_string() },
            action_record: Some(ActionRecord {
                task_id: "1".to_string(),
                request: ToolRequest::WriteFile { path: PathBuf::from("test.txt"), content: "data".to_string() },
                approval_status: ApprovalStatus::Pending,
                outcome: ExecutionOutcome::AwaitingApproval,
            }),
            verification: None,
        });

        resolve_pending_action(&agent, &mut trace, 0, true).unwrap();

        let record = trace.steps[0].action_record.as_ref().unwrap();
        assert_eq!(record.approval_status, ApprovalStatus::Approved);
        assert_eq!(record.outcome, ExecutionOutcome::Executed(ToolResult::Success));
        assert_eq!(fs::read_to_string(root.join("test.txt")).unwrap(), "data");
    }

    #[test]
    fn test_resolve_pending_action_deny() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let mut trace = ExecutionTrace::new();
        trace.steps.push(StepResult {
            model_response: ModelResponse { content: "Writing".to_string() },
            action_record: Some(ActionRecord {
                task_id: "1".to_string(),
                request: ToolRequest::WriteFile { path: PathBuf::from("test.txt"), content: "data".to_string() },
                approval_status: ApprovalStatus::Pending,
                outcome: ExecutionOutcome::AwaitingApproval,
            }),
            verification: None,
        });

        resolve_pending_action(&agent, &mut trace, 0, false).unwrap();

        let record = trace.steps[0].action_record.as_ref().unwrap();
        assert_eq!(record.approval_status, ApprovalStatus::Denied);
        assert_eq!(record.outcome, ExecutionOutcome::Denied);
        assert!(!root.join("test.txt").exists());
    }

    #[test]
    fn test_resolve_not_pending() {
        let agent = Agent::new();
        let mut trace = ExecutionTrace::new();
        trace.steps.push(StepResult {
            model_response: ModelResponse { content: "Reading".to_string() },
            action_record: Some(ActionRecord {
                task_id: "1".to_string(),
                request: ToolRequest::DiscoverFiles,
                approval_status: ApprovalStatus::Approved,
                outcome: ExecutionOutcome::Executed(ToolResult::Success),
            }),
            verification: None,
        });

        let result = resolve_pending_action(&agent, &mut trace, 0, true);
        assert_eq!(result, Err(Error::ActionNotPending));
    }
}
