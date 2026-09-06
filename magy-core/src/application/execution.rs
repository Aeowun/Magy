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
use crate::application::reasoning::run_reasoning_step;
use crate::application::task_lifecycle::complete_current_task;
use crate::application::verification_runner::run_verification;
use crate::domain::agent::{Agent, Event, State};
use crate::domain::model::{ExecutionOutcome, ExecutionTrace, ModelProvider};
use crate::domain::project::{Project, ProjectContext, ProjectPlan};
use crate::domain::tool::{ToolRequest, ToolResult};
use crate::Error;

/// Performs a bounded sequence of reasoning and execution steps.
pub fn run_execution_cycle(
    agent: &mut Agent,
    project: &mut Project,
    context: &ProjectContext,
    plan: &ProjectPlan,
    provider: &dyn ModelProvider,
    policy: &dyn ApprovalPolicy,
    system_prompt: &str,
    max_steps: usize,
    max_verifications: usize,
    verification_command: &str,
    trace: &mut ExecutionTrace,
) -> Result<(), Error> {
    trace.stopped_reason = String::new();
    let mut verification_count = 0;

    for i in 0..max_steps {
        let step = match run_reasoning_step(
            agent,
            context,
            plan,
            &trace.steps,
            provider,
            policy,
            system_prompt,
        ) {
            Ok(s) => s,
            Err(e) => {
                trace.stopped_reason = format!("Runtime error: {:?}", e);
                return Ok(());
            }
        };

        let record = step.action_record.as_ref();
        let is_task_complete = record
            .map(|r| r.request == ToolRequest::TaskComplete)
            .unwrap_or(false);

        if is_task_complete {
            let unresolved_action = trace.steps.iter().any(|previous| {
                previous
                    .action_record
                    .as_ref()
                    .map(|record| {
                        matches!(
                            record.outcome,
                            ExecutionOutcome::Denied
                                | ExecutionOutcome::AwaitingApproval
                                | ExecutionOutcome::Executed(ToolResult::Error(_))
                        )
                    })
                    .unwrap_or(false)
            });
            if unresolved_action {
                let mut rejected_step = step;
                if let Some(record) = rejected_step.action_record.as_mut() {
                    record.approval_status = crate::domain::model::ApprovalStatus::Denied;
                    record.outcome = ExecutionOutcome::Denied;
                }
                trace.steps.push(rejected_step);
                agent.transition(Event::TestsFailed)?;
                trace.stopped_reason =
                    "Task completion rejected: unresolved action remains".to_string();
                continue;
            }

            trace.steps.push(step);

            let ver_res = run_verification(agent, verification_command)?;

            if let Some(last_step) = trace.steps.last_mut() {
                last_step.verification = Some(ver_res.clone());
            }

            if ver_res.passed {
                complete_current_task(agent, project)?;
                trace.stopped_reason = "Task completed successfully".to_string();
                return Ok(());
            } else {
                agent.transition(Event::TestsFailed)?;
                verification_count += 1;

                if verification_count >= max_verifications {
                    trace.stopped_reason =
                        "Verification warning: maximum verification attempts reached".to_string();
                    return Ok(());
                }
                continue;
            }
        }

        let stop = match record {
            None => {
                if step.model_response.content.contains('{') {
                    trace.stopped_reason =
                        "Invalid model action; asking the model for a valid tool call".to_string();
                    false
                } else {
                    trace.stopped_reason = "Model stopped without action".to_string();
                    true
                }
            }
            Some(r) => match r.outcome {
                ExecutionOutcome::Denied => {
                    trace.stopped_reason =
                        "Action denied; asking the model for a different action".to_string();
                    false
                }
                ExecutionOutcome::AwaitingApproval => {
                    trace.stopped_reason = "Action requires approval".to_string();
                    true
                }
                ExecutionOutcome::Executed(ToolResult::Error(_)) => {
                    trace.stopped_reason = "Tool execution failed".to_string();
                    true
                }
                _ => false,
            },
        };

        trace.steps.push(step);

        if stop {
            return Ok(());
        }

        if i == max_steps - 1 {
            trace.stopped_reason = "Maximum steps reached".to_string();
            return Ok(());
        }

        if agent.state() != &State::Executing {
            trace.stopped_reason = "Agent is no longer in Executing state".to_string();
            return Ok(());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::approval::DefaultApprovalPolicy;
    use crate::application::context_assembly::assemble_project_context;
    use crate::application::planning::plan_execution;
    use crate::application::project_lifecycle::open_project;
    use crate::application::task_lifecycle::select_task;
    use crate::domain::model::{ModelRequest, ModelResponse};
    use crate::domain::project::TaskStatus;
    use std::fs;
    use tempfile::tempdir;

    struct MultiMockProvider {
        responses: std::cell::RefCell<Vec<Result<ModelResponse, Error>>>,
    }
    impl ModelProvider for MultiMockProvider {
        fn ask(&self, _req: ModelRequest) -> Result<ModelResponse, Error> {
            let mut resps = self.responses.borrow_mut();
            if resps.is_empty() {
                panic!(
                    "MultiMockProvider: no more responses! Request was: {:?}",
                    _req.history.len()
                );
            }
            resps.remove(0)
        }
    }

    #[test]
    fn test_run_execution_cycle_sequential() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();
        fs::write(root.join("input.txt"), "data").unwrap();

        let (mut agent, mut project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse {
                    content: "```json\n{\"tool\": \"read_file\", \"path\": \"input.txt\"}\n```"
                        .to_string(),
                }),
                Ok(ModelResponse {
                    content: "Done.".to_string(),
                }),
            ]),
        };

        let mut trace = ExecutionTrace::new();
        run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            &provider,
            &DefaultApprovalPolicy::default(),
            "S",
            5,
            1,
            "echo verify",
            &mut trace,
        )
        .unwrap();

        assert_eq!(trace.steps.len(), 2);
        assert_eq!(trace.stopped_reason, "Model stopped without action");
    }

    #[test]
    fn test_run_execution_cycle_pending_stop() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, mut project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse { content: "```json\n{\"tool\": \"write_file\", \"path\": \"out.txt\", \"content\": \"data\"}\n```".to_string() }),
            ]),
        };

        let mut trace = ExecutionTrace::new();
        run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            &provider,
            &DefaultApprovalPolicy::default(),
            "S",
            5,
            1,
            "echo verify",
            &mut trace,
        )
        .unwrap();

        assert_eq!(trace.steps.len(), 1);
        assert_eq!(trace.stopped_reason, "Action requires approval");
        assert_eq!(
            trace.steps[0].action_record.as_ref().unwrap().outcome,
            ExecutionOutcome::AwaitingApproval
        );
    }

    #[test]
    fn test_invalid_structured_action_is_retryable() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, mut project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();
        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse {
                    content: "{\"tool\":\"write_file\",\"path\":null,\"content\":\"x\",\"command\":\"task_complete\"}".to_string(),
                }),
                Ok(ModelResponse {
                    content: "{\"tool\":\"write_file\",\"path\":\"out.txt\",\"content\":\"x\",\"command\":null}".to_string(),
                }),
            ]),
        };

        let mut trace = ExecutionTrace::new();
        run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            &provider,
            &DefaultApprovalPolicy::default(),
            "S",
            2,
            1,
            "echo verify",
            &mut trace,
        )
        .unwrap();

        assert_eq!(trace.steps.len(), 2);
        assert!(trace.stopped_reason.contains("approval"));
        assert!(trace.steps[0].action_record.is_none());
        assert!(trace.steps[1].action_record.is_some());
    }

    #[test]
    fn test_task_complete_rejected_after_denied_required_action() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, mut project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();
        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse {
                    content: "{\"tool\":\"run_command\",\"command\":\"git init\",\"path\":null,\"content\":null}".to_string(),
                }),
                Ok(ModelResponse {
                    content: "{\"tool\":\"task_complete\",\"command\":null,\"path\":null,\"content\":null}".to_string(),
                }),
            ]),
        };

        let mut trace = ExecutionTrace::new();
        run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            &provider,
            &DefaultApprovalPolicy::default(),
            "S",
            2,
            1,
            "",
            &mut trace,
        )
        .unwrap();

        assert_eq!(project.tasks[0].status, TaskStatus::Open);
        assert!(trace.stopped_reason.starts_with("Task completion rejected"));
        assert_eq!(trace.steps.len(), 2);
    }

    #[test]
    fn test_run_execution_cycle_resume() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, mut project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse { content: "```json\n{\"tool\": \"write_file\", \"path\": \"res.txt\", \"content\": \"ok\"}\n```".to_string() }),
                Ok(ModelResponse { content: "Finished".to_string() }),
            ]),
        };

        let mut trace = ExecutionTrace::new();
        run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            &provider,
            &DefaultApprovalPolicy::default(),
            "S",
            5,
            1,
            "echo verify",
            &mut trace,
        )
        .unwrap();

        assert_eq!(trace.steps.len(), 1);
        assert_eq!(trace.stopped_reason, "Action requires approval");

        crate::application::approval::resolve_pending_action(&agent, &mut trace, 0, true).unwrap();
        assert_eq!(fs::read_to_string(root.join("res.txt")).unwrap(), "ok");

        run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            &provider,
            &DefaultApprovalPolicy::default(),
            "S",
            5,
            1,
            "echo verify",
            &mut trace,
        )
        .unwrap();

        assert_eq!(trace.steps.len(), 2);
        assert_eq!(trace.stopped_reason, "Model stopped without action");
        assert_eq!(trace.steps[1].model_response.content, "Finished");
    }

    #[test]
    fn test_run_execution_cycle_regression_flat_schema() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T1").unwrap();
        fs::write(root.join("main.rs"), "fn main() {}").unwrap();

        let (mut agent, mut project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        // Regression Flow:
        // 1. Model proposes write_file using flat schema.
        // 2. Verification command is run (fails first).
        // 3. Model proposes task_complete using flat schema.
        // 4. Verification passes.
        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse { content: "{\"tool\": \"write_file\", \"path\": \"main.rs\", \"content\": \"// fixed\", \"command\": null}".to_string() }),
                Ok(ModelResponse { content: "{\"tool\": \"task_complete\", \"path\": null, \"content\": null, \"command\": null}".to_string() }),
                Ok(ModelResponse { content: "{\"tool\": \"task_complete\", \"path\": null, \"content\": null, \"command\": null}".to_string() }),
            ]),
        };

        let mut trace = ExecutionTrace::new();
        let cmd = "echo verify"; // Always passes (exit code 0)

        // RUN 1: write_file. Note: DefaultApprovalPolicy marks write_file as Pending.
        run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            &provider,
            &DefaultApprovalPolicy::default(),
            "S",
            1,
            2,
            cmd,
            &mut trace,
        )
        .unwrap();

        assert_eq!(trace.stopped_reason, "Action requires approval");
        crate::application::approval::resolve_pending_action(&agent, &mut trace, 0, true).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("main.rs")).unwrap(),
            "// fixed"
        );

        // RUN 2: task_complete.
        run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            &provider,
            &DefaultApprovalPolicy::default(),
            "S",
            5,
            2,
            cmd,
            &mut trace,
        )
        .unwrap();

        assert_eq!(agent.state(), &State::Planning);
        assert_eq!(project.tasks[0].status, TaskStatus::Done);
        assert!(
            trace
                .steps
                .last()
                .unwrap()
                .verification
                .as_ref()
                .unwrap()
                .passed
        );
    }
}
