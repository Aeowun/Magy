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

use std::path::PathBuf;
use crate::Error;
use crate::domain::agent::State;
use crate::domain::model::{ModelProvider, ExecutionTrace, RunResult};
use crate::application::project_lifecycle::open_project;
use crate::application::context_assembly::assemble_project_context;
use crate::application::planning::plan_execution;
use crate::application::task_lifecycle::select_task;
use crate::application::execution::run_execution_cycle;
use crate::application::approval::ApprovalPolicy;

/// Coordinates the full agent workflow for a project.
pub fn run_project_workflow(
    root: PathBuf,
    provider: &dyn ModelProvider,
    policy: &dyn ApprovalPolicy,
    verification_command: &str,
    system_prompt: &str,
    max_steps_per_cycle: usize,
    max_verifications_per_task: usize,
    trace: &mut ExecutionTrace,
) -> Result<RunResult, Error> {
    // 1. Initialize session.
    let (mut agent, mut project) = open_project(root.clone())?;
    let mut completed_task_ids = Vec::new();

    loop {
        // 2. Assemble context and plan.
        let context = assemble_project_context(root.clone(), project.clone())?;
        let plan = plan_execution(&agent, &context)?;

        // 3. Check for project completion.
        let next_task = match plan.tasks.first() {
            Some(t) => t,
            None => {
                return Ok(RunResult {
                    project_completed: true,
                    completed_task_ids,
                    active_task_id: None,
                    stop_reason: "All tasks completed".to_string(),
                });
            }
        };

        // 4. Select task.
        let task_id = next_task.id.clone();
        select_task(&mut agent, &project, &task_id)?;

        // 5. Execute bounded cycle.
        run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            provider,
            policy,
            system_prompt,
            max_steps_per_cycle,
            max_verifications_per_task,
            verification_command,
            trace,
        )?;

        // 6. Inspect outcome.
        let stopped_reason = trace.stopped_reason.clone();

        // Did the task complete successfully during the cycle?
        // We know it did if the agent returned to Planning state.
        if agent.state() == &State::Planning {
            completed_task_ids.push(task_id);
            // Continue loop to next task.
            continue;
        }

        // Otherwise, we stopped for a reason that requires outside intervention or represents a limit.
        return Ok(RunResult {
            project_completed: false,
            completed_task_ids,
            active_task_id: Some(task_id),
            stop_reason: stopped_reason,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use crate::domain::model::{ModelResponse, ModelRequest, ExecutionOutcome};
    use crate::application::approval::DefaultApprovalPolicy;

    struct MultiMockProvider {
        responses: std::cell::RefCell<Vec<Result<ModelResponse, Error>>>,
    }
    impl ModelProvider for MultiMockProvider {
        fn ask(&self, _req: ModelRequest) -> Result<ModelResponse, Error> {
            let mut resps = self.responses.borrow_mut();
            if resps.is_empty() {
                return Ok(ModelResponse { content: "No more mock responses".to_string() });
            }
            resps.remove(0)
        }
    }

    #[test]
    fn test_run_project_workflow_success_multiple_tasks() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T1\n- [ ] T2").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                // Task 1
                Ok(ModelResponse { content: "```json\n{\"tool\": \"task_complete\"}\n```".to_string() }),
                // Task 2
                Ok(ModelResponse { content: "```json\n{\"tool\": \"task_complete\"}\n```".to_string() }),
            ]),
        };

        let mut trace = ExecutionTrace::new();
        let result = run_project_workflow(
            root,
            &provider,
            &DefaultApprovalPolicy,
            "echo verify",
            "S",
            5,
            1,
            &mut trace
        ).unwrap();

        assert!(result.project_completed);
        assert_eq!(result.completed_task_ids, vec!["1", "2"]);
        assert_eq!(result.stop_reason, "All tasks completed");
    }

    #[test]
    fn test_run_project_workflow_stop_on_approval() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T1").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse { content: "```json\n{\"tool\": \"write_file\", \"path\": \"a.txt\", \"content\": \"c\"}\n```".to_string() }),
            ]),
        };

        let mut trace = ExecutionTrace::new();
        let result = run_project_workflow(
            root,
            &provider,
            &DefaultApprovalPolicy,
            "echo verify",
            "S",
            5,
            1,
            &mut trace
        ).unwrap();

        assert!(!result.project_completed);
        assert_eq!(result.active_task_id, Some("1".to_string()));
        assert_eq!(result.stop_reason, "Action requires approval");
        assert_eq!(trace.steps[0].action_record.as_ref().unwrap().outcome, ExecutionOutcome::AwaitingApproval);
    }

    #[test]
    fn test_run_project_workflow_skip_completed() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [x] T1\n- [ ] T2").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse { content: "```json\n{\"tool\": \"task_complete\"}\n```".to_string() }),
            ]),
        };

        let mut trace = ExecutionTrace::new();
        let result = run_project_workflow(
            root,
            &provider,
            &DefaultApprovalPolicy,
            "echo verify",
            "S",
            5,
            1,
            &mut trace
        ).unwrap();

        assert!(result.project_completed);
        assert_eq!(result.completed_task_ids, vec!["2"]);
    }
}
