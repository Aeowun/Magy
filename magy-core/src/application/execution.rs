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
use crate::application::reasoning::run_reasoning_step;
use crate::application::task_lifecycle::complete_current_task;
use crate::application::verification_runner::run_verification;
use crate::domain::agent::{Agent, Event, State};
use crate::domain::model::{
    now_ms, ExecutionOutcome, ExecutionTrace, FailureReason, ModelProvider, RunState, StepResult,
};
use crate::domain::project::{Evidence, Project, ProjectContext, ProjectPlan};
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
    trace.start(max_steps);
    trace.stopped_reason.clear();

    let task_id = match agent.task() {
        Some(t) => t.id.clone(),
        None => {
            trace.run.stall();
            trace.stopped_reason = "No active task for execution cycle".to_string();
            return Ok(());
        }
    };

    let _project_task = project.tasks.iter().find(|t| t.id == task_id).ok_or_else(|| {
        Error::Internal(format!("Task {} not found in project", task_id))
    })?;

    if trace.run.state() == &RunState::Starting {
        trace.run.request_plan();
    }
    if trace.run.state() == &RunState::Planning
        || trace.run.state() == &RunState::AwaitingApproval
        || trace.run.state() == &RunState::Recovering
        || trace.run.state() == &RunState::Stalled
        || trace.run.state() == &RunState::ExecutingTask
    {
        trace.run.await_model();
    }
    let mut verification_count = 0;
    let mut current_context = context.clone();

    if max_steps == 0 {
        let _ = agent.transition(Event::FatalError);
        trace.run.stall();
        trace.stopped_reason = "Maximum steps reached".to_string();
        return Ok(());
    }

    for i in 0..max_steps {
        if trace.run.state() == &RunState::Recovering {
            trace.run.await_model();
        }
        let step = match run_reasoning_step(
            agent,
            &current_context,
            plan,
            &trace.steps,
            provider,
            policy,
            system_prompt,
        ) {
            Ok(s) => s,
            Err(e) => {
                let _ = agent.transition(Event::FatalError);
                trace.run.fail(failure_reason(&e));
                trace.stopped_reason = format!("Runtime error: {:?}", e);
                return Err(e);
            }
        };

        let record = step.action_record.as_ref();
        if record.is_some() {
            trace.run.execute_tool();
        }
        if let Some(record) = record {
            if has_repeated_action(&trace.steps, &record.request, 2) {
                trace.run.record_recovery_attempt(FailureReason::Internal(
                    "Repeated identical action without new evidence".to_string(),
                ));
                trace.steps.push(step);
                trace.run.stall();
                trace.stopped_reason =
                    "Run stalled: repeated identical action without progress".to_string();
                return Ok(());
            }
        }
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
                trace.run.stall();
                trace.stopped_reason =
                    "Task completion rejected: unresolved action remains".to_string();
                return Ok(());
            }

            if let Err(error) = agent.transition(Event::ActionDone) {
                let _ = agent.transition(Event::FatalError);
                trace
                    .run
                    .fail(FailureReason::Internal(format!("{:?}", error)));
                trace.stopped_reason = format!("Runtime error: {:?}", error);
                return Err(error);
            }
            trace.steps.push(step);

            trace.run.request_verification();
            trace.run.begin_verification();
            let ver_res = match run_verification(agent, verification_command) {
                Ok(result) => result,
                Err(e) => {
                    let _ = agent.transition(Event::FatalError);
                    trace.run.fail(failure_reason(&e));
                    trace.stopped_reason = format!("Runtime error: {:?}", e);
                    return Err(e);
                }
            };

            let evidence = Evidence {
                timestamp_ms: now_ms(),
                verifier: "runtime_verification".to_string(),
                passed: ver_res.passed,
                output: format!("STDOUT: {}\nSTDERR: {}", ver_res.stdout, ver_res.stderr),
            };

            if let Some(t) = project.tasks.iter_mut().find(|t| t.id == task_id) {
                t.evidence.push(evidence);
            }

            if let Some(last_step) = trace.steps.last_mut() {
                last_step.verification = Some(ver_res.clone());
            }

            if ver_res.passed {
                if let Err(e) = complete_current_task(agent, project) {
                    let _ = agent.transition(Event::FatalError);
                    trace.run.fail(failure_reason(&e));
                    trace.stopped_reason = format!("Runtime error: {:?}", e);
                    return Err(e);
                }
                trace.run.request_plan();
                trace.stopped_reason = "Task completed successfully".to_string();
                return Ok(());
            }

            if let Err(error) = agent.transition(Event::TestsFailed) {
                let _ = agent.transition(Event::FatalError);
                trace
                    .run
                    .fail(FailureReason::Internal(format!("{:?}", error)));
                trace.stopped_reason = format!("Runtime error: {:?}", error);
                return Err(error);
            }
            verification_count += 1;
            trace
                .run
                .record_verification_failure(verification_count, verification_command);
            trace.run.recover();

            if verification_count >= max_verifications {
                let _ = agent.transition(Event::FatalError);
                trace.run.stall();
                trace.stopped_reason =
                    "Verification warning: maximum verification attempts reached".to_string();
                return Ok(());
            }
            continue;
        }

        let stop = match record {
            None => {
                let _ = agent.transition(Event::FatalError);
                trace.run.stall();
                trace.stopped_reason = "Model stopped without action".to_string();
                true
            }
            Some(r) => match &r.outcome {
                ExecutionOutcome::Denied => {
                    if trace.run.record_recovery_attempt(FailureReason::Internal(
                        "Action was denied by policy".to_string(),
                    )) {
                        false
                    } else {
                        let _ = agent.transition(Event::FatalError);
                        trace.run.stall();
                        trace.stopped_reason = "Action denied".to_string();
                        true
                    }
                }
                ExecutionOutcome::AwaitingApproval => {
                    trace.run.await_approval();
                    trace.stopped_reason = "Action requires approval".to_string();
                    true
                }
                ExecutionOutcome::Executed(ToolResult::Error(message)) => {
                    let error = Error::ToolError(message.clone());
                    let _ = agent.transition(Event::FatalError);
                    trace.run.fail(FailureReason::Tool(message.clone()));
                    trace.stopped_reason = "Tool execution failed".to_string();
                    trace.steps.push(step);
                    return Err(error);
                }
                ExecutionOutcome::CompletionRequested => false,
                ExecutionOutcome::Executed(ToolResult::Success) => {
                    trace.run.await_model();
                    false
                }
                _ => false,
            },
        };

        trace.steps.push(step);

        if matches!(
            trace
                .steps
                .last()
                .and_then(|s| s.action_record.as_ref())
                .map(|r| &r.outcome),
            Some(ExecutionOutcome::Executed(ToolResult::Success))
        ) {
            if let Some(root) = agent.root() {
                match assemble_project_context(root.to_path_buf(), project.clone()) {
                    Ok(refreshed) => current_context = refreshed,
                    Err(error) => {
                        let _ = agent.transition(Event::FatalError);
                        trace
                            .run
                            .fail(FailureReason::Context(format!("{:?}", error)));
                        trace.stopped_reason =
                            format!("Runtime error refreshing project context: {:?}", error);
                        return Err(Error::ContextError(format!("{:?}", error)));
                    }
                }
            }
        }

        if stop {
            return Ok(());
        }

        if i == max_steps - 1 {
            let _ = agent.transition(Event::FatalError);
            trace.run.stall();
            trace.stopped_reason = "Maximum steps reached".to_string();
            return Ok(());
        }

        if agent.state() != &State::Executing {
            let error = Error::InvalidStateTransition;
            let _ = agent.transition(Event::FatalError);
            trace.run.fail(FailureReason::Internal(
                "Agent is no longer in Executing state".to_string(),
            ));
            trace.stopped_reason = "Agent is no longer in Executing state".to_string();
            return Err(error);
        }
    }

    let _ = agent.transition(Event::FatalError);
    trace.run.stall();
    trace.stopped_reason = "Maximum steps reached".to_string();
    Ok(())
}

fn has_repeated_action(
    history: &[StepResult],
    request: &ToolRequest,
    required_repeats: usize,
) -> bool {
    if required_repeats == 0 || history.len() < required_repeats {
        return false;
    }

    history.iter().rev().take(required_repeats).all(|step| {
        step.action_record
            .as_ref()
            .map(|record| &record.request == request)
            .unwrap_or(false)
    })
}

fn failure_reason(error: &Error) -> FailureReason {
    match error {
        Error::ModelError(message) => FailureReason::Model(message.clone()),
        Error::ParseError(message) => FailureReason::Parse(message.clone()),
        Error::ToolError(message) => FailureReason::Tool(message.clone()),
        Error::ContextError(message) => FailureReason::Context(message.clone()),
        other => FailureReason::Internal(format!("{:?}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::approval::DefaultApprovalPolicy;
    use crate::application::context_assembly::assemble_project_context;
    use crate::application::planning::plan_execution;
    use crate::application::project_lifecycle::open_project;
    use crate::application::task_lifecycle::select_task;
    use crate::domain::model::{ApprovalStatus, ModelResponse};
    use crate::domain::project::TaskStatus;
    use std::fs;
    use tempfile::tempdir;

    struct MultiMockProvider {
        responses: std::cell::RefCell<Vec<Result<ModelResponse, Error>>>,
    }
    impl ModelProvider for MultiMockProvider {
        fn ask(&self, _req: crate::domain::model::ModelRequest) -> Result<crate::domain::model::ModelResponse, Error> {
            let mut resps = self.responses.borrow_mut();
            if resps.is_empty() {
                panic!(
                    "MultiMockProvider: no more responses! Request was: {:?}",
                    _req.history.len()
                );
            }
            resps.remove(0)
        }
        fn plan(&self, _: crate::domain::model::PlannerRequest) -> Result<crate::domain::model::PlannerResponse, Error> {
            unreachable!()
        }
    }

    #[test]
    fn repeated_identical_actions_stall_without_unbounded_progress() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();
        fs::write(root.join("input.txt"), "data").unwrap();

        let (mut agent, mut project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();
        let response = Ok(ModelResponse {
            content: "```json\n{\"tool\": \"read_file\", \"path\": \"input.txt\"}\n```".to_string(),
        });
        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![response.clone(), response.clone(), response]),
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
            10,
            1,
            "echo verify",
            &mut trace,
        )
        .unwrap();

        assert_eq!(trace.run.state(), &RunState::Stalled);
        assert_eq!(trace.steps.len(), 3);
        assert!(trace.stopped_reason.contains("repeated identical action"));
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

        fs::write(root.join("res.txt"), "ok").unwrap();

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

        fs::write(root.join("res.txt"), "ok").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse { content: "```json\n{\"tool\": \"delete_file\", \"path\": \"out.txt\"}\n```".to_string() }),
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
    fn provider_error_leaves_a_terminal_failed_run() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, mut project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();
        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![Err(Error::ModelError(
                "provider unavailable".to_string(),
            ))]),
        };
        let mut trace = ExecutionTrace::new();

        let result = run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            &provider,
            &DefaultApprovalPolicy::default(),
            "S",
            3,
            1,
            "echo verify",
            &mut trace,
        );

        assert!(matches!(result, Err(Error::ModelError(_))));
        assert_eq!(agent.state(), &State::Failed);
        assert_eq!(trace.run.state(), &crate::domain::model::RunState::Failed);
        assert!(trace.validate());
    }

    #[test]
    fn tool_boundary_violation_stops_cycle() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, mut project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();
        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![Ok(ModelResponse {
                content: "```json\n{\"tool\":\"read_file\",\"path\":\"../outside\"}\n```"
                    .to_string(),
            })]),
        };
        let mut trace = ExecutionTrace::new();
        trace.run.start(3);
        trace.run.begin_planning();
        trace.run.await_model();
        trace.run.validate_plan();
        trace.run.commit_plan();
        trace.run.begin_task("1");

        let result = run_execution_cycle(
            &mut agent,
            &mut project,
            &context,
            &plan,
            &provider,
            &DefaultApprovalPolicy::default(),
            "S",
            3,
            1,
            "echo verify",
            &mut trace,
        );

        assert!(result.is_ok());
        // Boundary violations are now caught by policy and result in AwaitingApproval (soft guardrail)
        assert_eq!(trace.run.state(), &RunState::AwaitingApproval);
        assert_eq!(trace.steps.len(), 1);
        assert_eq!(trace.steps[0].action_record.as_ref().unwrap().approval_status, ApprovalStatus::Pending);
        assert!(trace.validate());
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
        let result = run_execution_cycle(
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
        );

        assert!(matches!(result, Err(Error::ParseError(_))));
        assert_eq!(trace.steps.len(), 0);
        assert_eq!(trace.run.state(), &crate::domain::model::RunState::Failed);
        assert!(!trace.run.state().is_active());
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

        fs::write(root.join("res.txt"), "ok").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse { content: "```json\n{\"tool\": \"delete_file\", \"path\": \"res.txt\"}\n```".to_string() }),
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
        assert!(!root.join("res.txt").exists());

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
        // 1. Model proposes delete_file using flat schema.
        // 2. Verification command is run (fails first).
        // 3. Model proposes task_complete using flat schema.
        // 4. Verification passes.
        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse { content: "{\"tool\": \"delete_file\", \"path\": \"main.rs\", \"from\": null, \"to\": null, \"content\": null, \"command\": null}".to_string() }),
                Ok(ModelResponse { content: "{\"tool\": \"task_complete\", \"path\": null, \"from\": null, \"to\": null, \"content\": null, \"command\": null}".to_string() }),
                Ok(ModelResponse { content: "{\"tool\": \"task_complete\", \"path\": null, \"from\": null, \"to\": null, \"content\": null, \"command\": null}".to_string() }),
            ]),
        };

        let mut trace = ExecutionTrace::new();
        let cmd = "echo verify"; // Always passes (exit code 0)

        // RUN 1: delete_file. Note: DefaultApprovalPolicy marks delete_file as Pending.
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
        assert!(!root.join("main.rs").exists());

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
