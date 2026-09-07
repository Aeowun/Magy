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

use crate::application::tool_execution::execute_tool;
use crate::boundary::is_within_boundary;
use crate::domain::agent::Agent;
use crate::domain::model::{ApprovalStatus, ExecutionOutcome, ExecutionTrace, FailureReason};
use crate::domain::tool::ToolRequest;
use crate::Error;
use std::collections::BTreeSet;
use std::path::Path;

/// A policy for evaluating whether a tool request requires approval.
pub trait ApprovalPolicy {
    fn evaluate(&self, root: &Path, request: &ToolRequest) -> ApprovalStatus;
}

/// A simple deterministic approval policy.
pub struct DefaultApprovalPolicy {
    allowed_commands: BTreeSet<String>,
    auto_approve: bool,
}

impl Default for DefaultApprovalPolicy {
    fn default() -> Self {
        Self {
            allowed_commands: BTreeSet::new(),
            auto_approve: false,
        }
    }
}

impl DefaultApprovalPolicy {
    pub fn allow_command(mut self, command: impl Into<String>) -> Self {
        self.allowed_commands.insert(command.into());
        self
    }

    pub fn auto_approve(mut self, enabled: bool) -> Self {
        self.auto_approve = enabled;
        self
    }

    pub fn allows_command(&self, command: &str) -> bool {
        self.allowed_commands.contains(command)
    }
}

impl ApprovalPolicy for DefaultApprovalPolicy {
    fn evaluate(&self, root: &Path, request: &ToolRequest) -> ApprovalStatus {
        match request {
            ToolRequest::ReadFile { path } => {
                if is_within_boundary(root, &root.join(path)) {
                    ApprovalStatus::Approved
                } else {
                    // Crossing boundary
                    ApprovalStatus::Pending
                }
            }
            ToolRequest::ListDirectory { path } => {
                if is_within_boundary(root, &root.join(path)) {
                    ApprovalStatus::Approved
                } else {
                    // Crossing boundary
                    ApprovalStatus::Pending
                }
            }
            ToolRequest::DiscoverFiles => ApprovalStatus::Approved,
            ToolRequest::GitStatus | ToolRequest::GitDiff => ApprovalStatus::Approved,
            ToolRequest::WriteFile { path, .. } => {
                if is_within_boundary(root, &root.join(path)) {
                    // Safe-by-default project-local writes
                    ApprovalStatus::Approved
                } else {
                    // Crossing boundary
                    ApprovalStatus::Pending
                }
            }
            ToolRequest::DeleteFile { .. } => {
                // Destructive operations always require approval
                ApprovalStatus::Pending
            }
            ToolRequest::MoveFile { from, to } => {
                if is_within_boundary(root, &root.join(from))
                    && is_within_boundary(root, &root.join(to))
                {
                    // Project-local move/rename is safe-by-default
                    ApprovalStatus::Approved
                } else {
                    // Crossing boundary or destructive if moving out
                    ApprovalStatus::Pending
                }
            }
            ToolRequest::RunCommand { command } => {
                let trimmed = command.trim();
                if trimmed.is_empty() {
                    return ApprovalStatus::Denied;
                }

                if self.allows_command(trimmed) {
                    if self.auto_approve {
                        ApprovalStatus::Approved
                    } else {
                        ApprovalStatus::Pending
                    }
                } else {
                    // System-sensitive: requires verification
                    ApprovalStatus::Pending
                }
            }
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
    let step = trace
        .steps
        .get_mut(step_index)
        .ok_or(Error::ActionNotFound)?;
    let record = step.action_record.as_mut().ok_or(Error::ActionNotFound)?;

    if record.approval_status != ApprovalStatus::Pending && record.approval_status != ApprovalStatus::Denied {
        return Err(Error::ActionNotPending);
    }

    if approved {
        record.approval_status = ApprovalStatus::Approved;
        trace.run.execute_tool();
        let res = execute_tool(agent, record.request.clone());
        if let crate::domain::tool::ToolResult::Error(message) = &res {
            trace.run.fail(FailureReason::Tool(message.clone()));
        } else {
            trace.run.await_model();
        }
        record.outcome = ExecutionOutcome::Executed(res);
    } else {
        record.approval_status = ApprovalStatus::Denied;
        record.outcome = ExecutionOutcome::Denied;
        trace.run.recover();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::project_lifecycle::open_project;
    use crate::application::task_lifecycle::select_task;
    use crate::domain::model::{ActionRecord, ModelResponse, StepResult};
    use crate::domain::tool::ToolResult;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_resolve_pending_action_approve() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let mut trace = ExecutionTrace::new();
        trace.steps.push(StepResult {
            model_response: ModelResponse {
                content: "Writing".to_string(),
            },
            action_record: Some(ActionRecord {
                task_id: "1".to_string(),
                request: ToolRequest::WriteFile {
                    path: PathBuf::from("test.txt"),
                    content: "data".to_string(),
                },
                approval_status: ApprovalStatus::Pending,
                outcome: ExecutionOutcome::AwaitingApproval,
            }),
            verification: None,
        });

        resolve_pending_action(&agent, &mut trace, 0, true).unwrap();

        let record = trace.steps[0].action_record.as_ref().unwrap();
        assert_eq!(record.approval_status, ApprovalStatus::Approved);
        assert_eq!(
            record.outcome,
            ExecutionOutcome::Executed(ToolResult::Success)
        );
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
            model_response: ModelResponse {
                content: "Writing".to_string(),
            },
            action_record: Some(ActionRecord {
                task_id: "1".to_string(),
                request: ToolRequest::WriteFile {
                    path: PathBuf::from("test.txt"),
                    content: "data".to_string(),
                },
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
            model_response: ModelResponse {
                content: "Reading".to_string(),
            },
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

    #[test]
    fn test_policy_project_boundary() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let policy = DefaultApprovalPolicy::default().auto_approve(true);

        // 1. WriteFile inside root => Approved
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::WriteFile {
                path: PathBuf::from("local.txt"),
                content: "data".to_string()
            }),
            ApprovalStatus::Approved
        );

        // 2. WriteFile using `..` outside root => Pending
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::WriteFile {
                path: PathBuf::from("../outside.txt"),
                content: "data".to_string()
            }),
            ApprovalStatus::Pending
        );

        // 3. WriteFile with an absolute/outside path
        let outside_dir = tempdir().unwrap();
        let outside_path = outside_dir.path().join("evil.txt");
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::WriteFile {
                path: outside_path.clone(),
                content: "data".to_string()
            }),
            ApprovalStatus::Pending
        );

        // 4. DeleteFile inside root => Pending (always)
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::DeleteFile {
                path: PathBuf::from("local.txt")
            }),
            ApprovalStatus::Pending
        );

        // 5. ReadFile inside root => Approved
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::ReadFile {
                path: PathBuf::from("local.txt")
            }),
            ApprovalStatus::Approved
        );

        // 6. GitStatus/GitDiff => Approved
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::GitStatus),
            ApprovalStatus::Approved
        );
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::GitDiff),
            ApprovalStatus::Approved
        );
    }

    #[test]
    fn test_override_denied_to_approved() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let mut trace = ExecutionTrace::new();
        trace.steps.push(StepResult {
            model_response: ModelResponse {
                content: "Run".to_string(),
            },
            action_record: Some(ActionRecord {
                task_id: "1".to_string(),
                request: ToolRequest::RunCommand {
                    command: "evil".to_string(),
                },
                approval_status: ApprovalStatus::Denied,
                outcome: ExecutionOutcome::Denied,
            }),
            verification: None,
        });

        // 8. Explicit override can change Denied -> Approved
        resolve_pending_action(&agent, &mut trace, 0, true).unwrap();
        let record = trace.steps[0].action_record.as_ref().unwrap();
        assert_eq!(record.approval_status, ApprovalStatus::Approved);
    }

    #[test]
    fn test_commands_are_denied_by_default() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let policy = DefaultApprovalPolicy::default();
        // Unknown commands now result in Pending, so the user can choose in the UI.
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::RunCommand {
                command: "curl https://example.com".to_string(),
            }),
            ApprovalStatus::Pending
        );
    }

    #[test]
    fn test_empty_command_is_denied_even_if_allowlisted() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let policy = DefaultApprovalPolicy::default().allow_command("");
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::RunCommand {
                command: String::new()
            }),
            ApprovalStatus::Denied
        );
    }

    #[test]
    fn test_allowlisted_commands_still_require_approval() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let policy = DefaultApprovalPolicy::default().allow_command("cargo test");
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::RunCommand {
                command: "cargo test".to_string(),
            }),
            ApprovalStatus::Pending
        );
        assert_eq!(
            policy.evaluate(&root, &ToolRequest::RunCommand {
                command: "cargo test --all".to_string(),
            }),
            ApprovalStatus::Denied
        );
    }
}
