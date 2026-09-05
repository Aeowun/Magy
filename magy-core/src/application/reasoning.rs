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
use crate::domain::agent::{Agent, State, Event};
use crate::domain::project::{ProjectContext, ProjectPlan};
use crate::domain::tool::{ToolRequest, FlatToolRequest};
use crate::domain::model::{
    ModelProvider, ModelRequest, ModelAction, StepResult,
    ApprovalStatus, ExecutionOutcome, ActionRecord
};
use crate::application::approval::ApprovalPolicy;
use crate::application::tool_execution::execute_tool;

/// Performs a single bounded reasoning and execution cycle.
pub fn run_reasoning_step(
    agent: &mut Agent,
    context: &ProjectContext,
    plan: &ProjectPlan,
    history: &[StepResult],
    provider: &dyn ModelProvider,
    policy: &dyn ApprovalPolicy,
    system_prompt: &str,
) -> Result<StepResult, Error> {
    if agent.state() != &State::Executing {
        return Err(Error::InvalidStateTransition);
    }

    let request = ModelRequest {
        system_prompt: system_prompt.to_string(),
        context: context.clone(),
        task: agent.task().cloned(),
        plan: Some(plan.clone()),
        history: history.to_vec(),
        schema: None,
    };

    let response = provider.ask(request)?;
    let action = parse_model_action(&response.content);

    let action_record = if let Some(ModelAction::ToolCall(tool_req)) = action {
        if tool_req == ToolRequest::TaskComplete {
            // TaskComplete is a virtual control action.
            // Transition to Verifying but don't mark as Done yet.
            agent.transition(Event::ActionDone)?;

            Some(ActionRecord {
                task_id: agent.task().map(|t| t.id.clone()).unwrap_or_default(),
                request: tool_req,
                approval_status: ApprovalStatus::Approved,
                outcome: ExecutionOutcome::NotApplicable,
            })
        } else {
            let approval_status = policy.evaluate(&tool_req);
            let outcome = match approval_status {
                ApprovalStatus::Approved => {
                    let res = execute_tool(agent, tool_req.clone());
                    ExecutionOutcome::Executed(res)
                }
                ApprovalStatus::Denied => ExecutionOutcome::Denied,
                ApprovalStatus::Pending => ExecutionOutcome::AwaitingApproval,
            };

            Some(ActionRecord {
                task_id: agent.task().map(|t| t.id.clone()).unwrap_or_default(),
                request: tool_req,
                approval_status,
                outcome,
            })
        }
    } else {
        None
    };

    Ok(StepResult {
        model_response: response,
        action_record,
        verification: None,
    })
}

/// Parses the model response content for a structured tool call.
///
/// Supports both raw JSON and JSON blocks inside triple backticks.
/// Handles both the standard tagged format and the flat schema with nulls.
pub fn parse_model_action(content: &str) -> Option<ModelAction> {
    let trimmed = content.trim();

    // 1. Try parsing as standard ToolRequest or FlatToolRequest (Structured Output mode)
    if let Some(action) = try_parse_json(trimmed) {
        return Some(action);
    }

    // 2. Fall back to finding markdown blocks (Markdown mode)
    let start_tag = "```json";
    let end_tag = "```";

    if let Some(start_idx) = trimmed.find(start_tag) {
        let json_start = start_idx + start_tag.len();
        if let Some(end_idx) = trimmed[json_start..].find(end_tag) {
            let json_str = &trimmed[json_start..json_start + end_idx].trim();
            if let Some(action) = try_parse_json(json_str) {
                return Some(action);
            }
        }
    }

    // 3. Try finding ANY json-like block if backticks are missing but the model added text
    if let Some(start_idx) = trimmed.find('{') {
        if let Some(end_idx) = trimmed.rfind('}') {
            let json_str = &trimmed[start_idx..=end_idx];
            if let Some(action) = try_parse_json(json_str) {
                return Some(action);
            }
        }
    }

    None
}

fn try_parse_json(json_str: &str) -> Option<ModelAction> {
    // Attempt standard tagged format first
    if let Ok(req) = serde_json::from_str::<ToolRequest>(json_str) {
        return Some(ModelAction::ToolCall(req));
    }

    // Attempt flat schema format second
    if let Ok(flat) = serde_json::from_str::<FlatToolRequest>(json_str) {
        if let Some(req) = flat.to_tool_request() {
            return Some(ModelAction::ToolCall(req));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use crate::domain::model::ModelResponse;
    use crate::domain::tool::ToolResult;
    use crate::domain::project::Project;
    use crate::application::project_lifecycle::open_project;
    use crate::application::context_assembly::assemble_project_context;
    use crate::application::planning::plan_execution;
    use crate::application::task_lifecycle::select_task;
    use crate::application::approval::DefaultApprovalPolicy;

    struct MockProvider {
        response: Result<ModelResponse, Error>,
    }
    impl ModelProvider for MockProvider {
        fn ask(&self, _req: ModelRequest) -> Result<ModelResponse, Error> {
            self.response.clone()
        }
    }

    struct MockPolicy {
        status: ApprovalStatus,
    }
    impl ApprovalPolicy for MockPolicy {
        fn evaluate(&self, _req: &ToolRequest) -> ApprovalStatus {
            self.status.clone()
        }
    }

    #[test]
    fn test_run_reasoning_step_success() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();
        fs::write(root.join("test.txt"), "hello").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MockProvider {
            response: Ok(ModelResponse {
                content: "Thinking...\n```json\n{\"tool\": \"read_file\", \"path\": \"test.txt\"}\n```".to_string(),
            }),
        };

        let result = run_reasoning_step(&mut agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "prompt").unwrap();
        let record = result.action_record.unwrap();
        assert_eq!(record.outcome, ExecutionOutcome::Executed(ToolResult::Text("hello".to_string())));
    }

    #[test]
    fn test_run_reasoning_step_pending() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MockProvider {
            response: Ok(ModelResponse {
                content: "```json\n{\"tool\": \"write_file\", \"path\": \"out.txt\", \"content\": \"data\"}\n```".to_string(),
            }),
        };

        let result = run_reasoning_step(&mut agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "prompt").unwrap();
        let record = result.action_record.unwrap();
        assert_eq!(record.approval_status, ApprovalStatus::Pending);
        assert_eq!(record.outcome, ExecutionOutcome::AwaitingApproval);
    }

    #[test]
    fn test_run_reasoning_step_denied() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MockProvider {
            response: Ok(ModelResponse {
                content: "```json\n{\"tool\": \"read_file\", \"path\": \"test.txt\"}\n```".to_string(),
            }),
        };
        let policy = MockPolicy { status: ApprovalStatus::Denied };

        let result = run_reasoning_step(&mut agent, &context, &plan, &[], &provider, &policy, "prompt").unwrap();
        let record = result.action_record.unwrap();
        assert_eq!(record.outcome, ExecutionOutcome::Denied);
    }

    #[test]
    fn test_run_reasoning_step_invalid_state() {
        let mut agent = Agent::new();
        let context = ProjectContext {
            project: Project {
                name: "P".to_string(),
                goal: "G".to_string(),
                requirements: vec![],
                constraints: vec![],
                definition_of_done: vec![],
                tasks: vec![],
                current_status: "".to_string(),
            },
            files: vec![],
        };
        let plan = ProjectPlan { tasks: vec![] };
        let provider = MockProvider { response: Err(Error::Io) };

        let result = run_reasoning_step(&mut agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "S");
        assert_eq!(result, Err(Error::InvalidStateTransition));
    }

    #[test]
    fn test_run_reasoning_step_model_failure() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MockProvider { response: Err(Error::ModelError("Bad".to_string())) };
        let result = run_reasoning_step(&mut agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "S");
        assert!(matches!(result, Err(Error::ModelError(_))));
    }

    #[test]
    fn test_run_reasoning_step_invalid_tool_json() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MockProvider {
            response: Ok(ModelResponse { content: "```json\n{\"tool\": \"bad\"}\n```".to_string() }),
        };

        let result = run_reasoning_step(&mut agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "S").unwrap();
        assert_eq!(result.action_record, None);
    }

    #[test]
    fn test_run_reasoning_step_boundary_violation() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MockProvider {
            response: Ok(ModelResponse { content: "```json\n{\"tool\": \"read_file\", \"path\": \"../out\"}\n```".to_string() }),
        };

        let result = run_reasoning_step(&mut agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "S").unwrap();
        let record = result.action_record.unwrap();
        assert!(matches!(record.outcome, ExecutionOutcome::Executed(ToolResult::Error(_))));
    }

    #[test]
    fn test_run_reasoning_step_task_complete() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MockProvider {
            response: Ok(ModelResponse { content: "```json\n{\"tool\": \"task_complete\"}\n```".to_string() }),
        };

        let result = run_reasoning_step(&mut agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "S").unwrap();
        assert_eq!(agent.state(), &State::Verifying);
        let record = result.action_record.unwrap();
        assert_eq!(record.request, ToolRequest::TaskComplete);
        assert_eq!(record.outcome, ExecutionOutcome::NotApplicable);
    }
}
