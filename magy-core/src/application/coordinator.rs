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

use crate::application::approval::ApprovalPolicy;
use crate::application::context_assembly::assemble_project_context;
use crate::application::execution::run_execution_cycle;
use crate::application::planning::{plan_execution, plan_project};
use crate::application::project_lifecycle::{open_project, save_project};
use crate::application::task_lifecycle::select_task;
use crate::domain::model::{
    now_ms, ExecutionTrace, FailureReason, ModelProvider, RunResult, RunState,
};
use crate::Error;
use std::path::PathBuf;

/// Coordinates the full agent workflow for a project.
pub fn run_project_workflow(
    root: PathBuf,
    planner: &dyn ModelProvider,
    executor: &dyn ModelProvider,
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
    trace.start(max_verifications_per_task);
    trace.run.request_plan();

    loop {
        // 2. Assemble context.
        let context = match assemble_project_context(root.clone(), project.clone()) {
            Ok(context) => context,
            Err(error) => {
                trace
                    .run
                    .fail(FailureReason::Context(format!("{:?}", error)));
                return Err(error);
            }
        };

        // 3. Planning Phase
        if project.tasks.is_empty() {
            trace.run.begin_planning();
            trace.run.await_model();
            let planner_response = match plan_project(&project, &context, planner, system_prompt) {
                Ok(resp) => resp,
                Err(error) => {
                    trace.run.fail(match &error {
                        Error::ModelError(message) => FailureReason::Model(message.clone()),
                        _ => FailureReason::Internal(format!("{:?}", error)),
                    });
                    return Err(error);
                }
            };
            trace.run.validate_plan();
            project.tasks = planner_response.tasks;
            project.plan_version = planner_response.plan_version;
            project.plan_created_at_ms = Some(now_ms());

            if let Err(e) = save_project(&root, &project) {
                 trace.run.fail(FailureReason::Internal(format!("{:?}", e)));
                 return Err(e);
            }
            trace.run.commit_plan();
        }

        // Re-assemble context AFTER planning to ensure the new tasks are available in the context
        // passed to plan_execution and subsequent steps.
        let context = match assemble_project_context(root.clone(), project.clone()) {
            Ok(context) => context,
            Err(error) => {
                trace
                    .run
                    .fail(FailureReason::Context(format!("{:?}", error)));
                return Err(error);
            }
        };

        let plan = match plan_execution(&agent, &context) {
            Ok(plan) => plan,
            Err(error) => {
                trace.run.fail(FailureReason::Internal(format!("{:?}", error)));
                return Err(error);
            }
        };

        // 4. Check for project completion.
        let next_task = match plan.tasks.first() {
            Some(t) => t,
            None => {
                trace.run.complete(completed_task_ids.clone());
                return Ok(RunResult {
                    project_completed: true,
                    completed_task_ids: completed_task_ids.clone(),
                    active_task_id: None,
                    state: RunState::Completed,
                    outcome: trace.run.outcome().cloned(),
                    recovery: trace.run.recovery().clone(),
                    stop_reason: "All tasks completed".to_string(),
                    agent,
                    project,
                });
            }
        };

        // 5. Select task.
        let task_id = next_task.id.clone();
        if let Err(error) = select_task(&mut agent, &project, &task_id) {
            trace
                .run
                .fail(FailureReason::Internal(format!("{:?}", error)));
            return Err(error);
        }
        trace.run.begin_task(&task_id);

        // 6. Execute bounded cycle.
        run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            executor,
            policy,
            system_prompt,
            max_steps_per_cycle,
            max_verifications_per_task,
            verification_command,
            trace,
        )?;

        // 7. Inspect outcome.
        if project.tasks.iter().any(|task| {
            task.id == task_id && task.status == crate::domain::project::TaskStatus::Done
        }) {
            completed_task_ids.push(task_id);
            continue;
        }

        return Ok(RunResult {
            project_completed: false,
            completed_task_ids,
            active_task_id: Some(task_id),
            state: trace.run.state().clone(),
            outcome: trace.run.outcome().cloned(),
            recovery: trace.run.recovery().clone(),
            stop_reason: trace.stopped_reason.clone(),
            agent,
            project,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::approval::DefaultApprovalPolicy;
    use crate::domain::model::ExecutionOutcome;
    use std::fs;
    use tempfile::tempdir;

    struct MultiMockProvider {
        responses: std::cell::RefCell<Vec<Result<crate::domain::model::ModelResponse, Error>>>,
        plans: std::cell::RefCell<Vec<Result<crate::domain::model::PlannerResponse, Error>>>,
    }
    impl ModelProvider for MultiMockProvider {
        fn ask(&self, _req: crate::domain::model::ModelRequest) -> Result<crate::domain::model::ModelResponse, Error> {
            let mut resps = self.responses.borrow_mut();
            if resps.is_empty() {
                return Ok(crate::domain::model::ModelResponse {
                    content: "No more mock responses".to_string(),
                });
            }
            resps.remove(0)
        }
        fn plan(&self, _req: crate::domain::model::PlannerRequest) -> Result<crate::domain::model::PlannerResponse, Error> {
            let mut plans = self.plans.borrow_mut();
            if plans.is_empty() {
                panic!("MultiMockProvider: no more mock plans");
            }
            plans.remove(0)
        }
    }

    #[test]
    fn test_run_project_workflow_success_multiple_tasks() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // Project.md initially contains no tasks to trigger LLM planning
        fs::write(
            root.join("Project.md"),
            "P\n\nGoal\nG\n\nTasks\n",
        )
        .unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                // Task 1 execution
                Ok(crate::domain::model::ModelResponse {
                    content: "```json\n{\"tool\": \"task_complete\"}\n```".to_string(),
                }),
                // Task 2 execution
                Ok(crate::domain::model::ModelResponse {
                    content: "```json\n{\"tool\": \"task_complete\"}\n```".to_string(),
                }),
            ]),
            plans: std::cell::RefCell::new(vec![
                Ok(crate::domain::model::PlannerResponse {
                    plan_version: 1,
                    tasks: vec![
                        crate::domain::project::ProjectTask {
                            id: "1".to_string(),
                            description: "T1".to_string(),
                            status: crate::domain::project::TaskStatus::Open,
                            acceptance_criteria: vec!["C1".to_string()],
                            evidence: vec![],
                        },
                        crate::domain::project::ProjectTask {
                            id: "2".to_string(),
                            description: "T2".to_string(),
                            status: crate::domain::project::TaskStatus::Open,
                            acceptance_criteria: vec!["C2".to_string()],
                            evidence: vec![],
                        },
                    ],
                })
            ]),
        };

        let mut trace = ExecutionTrace::new();
        let result = run_project_workflow(
            root,
            &provider,
            &provider,
            &DefaultApprovalPolicy::default(),
            "echo verify",
            "S",
            5,
            1,
            &mut trace,
        )
        .unwrap();

        assert!(result.project_completed);
        assert_eq!(result.completed_task_ids, vec!["1", "2"]);
        assert_eq!(result.stop_reason, "All tasks completed");
        assert_eq!(result.agent.state(), &crate::domain::agent::State::Planning);
    }

    #[test]
    fn test_run_project_workflow_stop_on_approval() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T1").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(crate::domain::model::ModelResponse { content: "```json\n{\"tool\": \"write_file\", \"path\": \"a.txt\", \"content\": \"c\"}\n```".to_string() }),
            ]),
            plans: std::cell::RefCell::new(vec![]),
        };

        let mut trace = ExecutionTrace::new();
        let result = run_project_workflow(
            root,
            &provider,
            &provider,
            &DefaultApprovalPolicy::default(),
            "echo verify",
            "S",
            5,
            1,
            &mut trace,
        )
        .unwrap();

        assert!(!result.project_completed);
        assert_eq!(result.active_task_id, Some("1".to_string()));
        assert_eq!(result.stop_reason, "Action requires approval");
        assert_eq!(
            trace.steps[0].action_record.as_ref().unwrap().outcome,
            ExecutionOutcome::AwaitingApproval
        );
    }

    #[test]
    fn test_run_project_workflow_skip_completed() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(
            root.join("Project.md"),
            "P\n\nGoal\nG\n\nTasks\n- [x] T1\n- [ ] T2",
        )
        .unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![Ok(crate::domain::model::ModelResponse {
                content: "```json\n{\"tool\": \"task_complete\"}\n```".to_string(),
            })]),
            plans: std::cell::RefCell::new(vec![]),
        };

        let mut trace = ExecutionTrace::new();
        let result = run_project_workflow(
            root,
            &provider,
            &provider,
            &DefaultApprovalPolicy::default(),
            "echo verify",
            "S",
            5,
            1,
            &mut trace,
        )
        .unwrap();

        assert!(result.project_completed);
        assert_eq!(result.completed_task_ids, vec!["2"]);
    }
}
