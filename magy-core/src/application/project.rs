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

use std::path::{Path, PathBuf};
use crate::Error;
use crate::domain::agent::{Agent, Event, Task, State};
use crate::domain::project::{
    Project, TaskStatus, parse_project_md, serialize_project_md,
    ProjectContext, FileContext, FileContent, ProjectPlan
};
use crate::domain::tool::{ToolRequest, ToolResult};
use crate::domain::model::{
    ModelProvider, ModelRequest, ModelResponse, ModelAction, StepResult,
    ApprovalStatus, ExecutionOutcome, ActionRecord
};
use crate::infrastructure::fs::{read_file, write_file, list_directory, discover_files};
use crate::infrastructure::cmd::run_project_command;

/// The summary of a bounded execution cycle.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionTrace {
    pub steps: Vec<StepResult>,
    pub stopped_reason: String,
}

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
        }
    }
}

/// Coordinates the opening of a project: reading the file, parsing it,
/// and initializing the agent run.
pub fn open_project(root: PathBuf) -> Result<(Agent, Project), Error> {
    // 1. Read Project.md from the root.
    let content = read_file(&root, Path::new("Project.md"))?;

    // 2. Parse the content into the Project domain model.
    let project = parse_project_md(&content)?;

    // 3. Create a new Agent.
    let mut agent = Agent::new();

    // 4. Start the Agent with the project root.
    agent.transition(Event::Start(root))?;

    // 5. Return the coordinated result.
    Ok((agent, project))
}

/// Performs a bounded sequence of reasoning and execution steps.
pub fn run_execution_cycle(
    agent: &Agent,
    context: &ProjectContext,
    plan: &ProjectPlan,
    provider: &dyn ModelProvider,
    policy: &dyn ApprovalPolicy,
    system_prompt: &str,
    max_steps: usize,
) -> Result<ExecutionTrace, Error> {
    let mut trace = ExecutionTrace {
        steps: Vec::new(),
        stopped_reason: String::new(),
    };

    for i in 0..max_steps {
        // 1. Perform one reasoning/execution step.
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
                return Ok(trace);
            }
        };

        let stop = match &step.action_record {
            None => {
                trace.stopped_reason = "Model stopped without action".to_string();
                true
            }
            Some(record) => match record.outcome {
                ExecutionOutcome::Denied => {
                    trace.stopped_reason = "Action denied".to_string();
                    true
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
            return Ok(trace);
        }

        if i == max_steps - 1 {
            trace.stopped_reason = "Maximum steps reached".to_string();
            return Ok(trace);
        }

        // 3. Re-verify agent state.
        if agent.state() != &State::Executing {
            trace.stopped_reason = "Agent is no longer in Executing state".to_string();
            return Ok(trace);
        }
    }

    Ok(trace)
}

/// Performs a single bounded reasoning and execution cycle.
pub fn run_reasoning_step(
    agent: &Agent,
    context: &ProjectContext,
    plan: &ProjectPlan,
    history: &[StepResult],
    provider: &dyn ModelProvider,
    policy: &dyn ApprovalPolicy,
    system_prompt: &str,
) -> Result<StepResult, Error> {
    // 1. Ensure the agent is in the Executing state.
    if agent.state() != &State::Executing {
        return Err(Error::InvalidStateTransition);
    }

    // 2. Ask the model for the next action.
    let request = ModelRequest {
        system_prompt: system_prompt.to_string(),
        context: context.clone(),
        task: agent.task().cloned(),
        plan: Some(plan.clone()),
        history: history.to_vec(),
    };

    let response = provider.ask(request)?;

    // 3. Parse the model's response for a tool call.
    let action = parse_model_action(&response.content);

    // 4. Evaluate approval and execute if permitted.
    let action_record = if let Some(ModelAction::ToolCall(tool_req)) = action {
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
    } else {
        None
    };

    Ok(StepResult {
        model_response: response,
        action_record,
    })
}

/// Parses the model response content for a structured tool call.
///
/// Currently supports a JSON block inside triple backticks.
fn parse_model_action(content: &str) -> Option<ModelAction> {
    let start_tag = "```json";
    let end_tag = "```";

    if let Some(start_idx) = content.find(start_tag) {
        let json_start = start_idx + start_tag.len();
        if let Some(end_idx) = content[json_start..].find(end_tag) {
            let json_str = &content[json_start..json_start + end_idx].trim();
            if let Ok(req) = serde_json::from_str::<ToolRequest>(json_str) {
                return Some(ModelAction::ToolCall(req));
            }
        }
    }

    None
}

/// Executes a tool request within the project boundary.
///
/// The agent must be in the Executing state.
pub fn execute_tool(agent: &Agent, request: ToolRequest) -> ToolResult {
    // 1. Ensure the agent is in the Executing state.
    if agent.state() != &State::Executing {
        return ToolResult::Error("Agent is not in Executing state".to_string());
    }

    // 2. Get the project root.
    let root = match agent.root() {
        Some(r) => r,
        None => return ToolResult::Error("Agent has no project root".to_string()),
    };

    // 3. Execute the requested tool using existing infrastructure.
    match request {
        ToolRequest::ReadFile { path } => {
            match read_file(root, &path) {
                Ok(content) => ToolResult::Text(content),
                Err(e) => ToolResult::Error(format!("Read error: {:?}", e)),
            }
        }
        ToolRequest::WriteFile { path, content } => {
            match write_file(root, &path, &content) {
                Ok(()) => ToolResult::Success,
                Err(e) => ToolResult::Error(format!("Write error: {:?}", e)),
            }
        }
        ToolRequest::ListDirectory { path } => {
            match list_directory(root, &path) {
                Ok(entries) => ToolResult::Entries(entries),
                Err(e) => ToolResult::Error(format!("List error: {:?}", e)),
            }
        }
        ToolRequest::DiscoverFiles => {
            match discover_files(root) {
                Ok(paths) => ToolResult::Paths(paths),
                Err(e) => ToolResult::Error(format!("Discovery error: {:?}", e)),
            }
        }
        ToolRequest::RunCommand { command } => {
            match run_project_command(root, &command) {
                Ok(out) => ToolResult::Command(out),
                Err(e) => ToolResult::Error(format!("Command error: {:?}", e)),
            }
        }
    }
}

/// Produces a deterministic execution plan from the project context.
///
/// The plan contains all Open tasks in their original order.
pub fn plan_execution(agent: &Agent, context: &ProjectContext) -> Result<ProjectPlan, Error> {
    // 1. Ensure the agent is in the Planning state.
    if agent.state() != &State::Planning {
        return Err(Error::InvalidStateTransition);
    }

    // 2. Identify open tasks.
    let open_tasks: Vec<_> = context.project.tasks.iter()
        .filter(|t| t.status == TaskStatus::Open)
        .cloned()
        .collect();

    // 3. Deterministic ordering.
    // Project tasks IDs are derived from their 1-based position in the list,
    // so sorting by ID (textually or numerically) preserves the Project.md order.
    // Since they were collected from an ordered iterator, they are already in order,
    // but we ensure it explicitly by ID if needed. Here, stable collection is sufficient.

    Ok(ProjectPlan {
        tasks: open_tasks,
    })
}

/// Assembles the complete project context: model and file tree with content.
pub fn assemble_project_context(root: PathBuf, project: Project) -> Result<ProjectContext, Error> {
    let paths = discover_files(&root)?;
    let mut files = Vec::new();

    for rel_path in paths {
        let abs_path = root.join(&rel_path);

        // Use symlink_metadata to avoid following links.
        let md = std::fs::symlink_metadata(&abs_path).map_err(|_| Error::Io)?;

        let content = if md.is_dir() {
            FileContent::Directory
        } else if md.is_file() {
            match read_file(&root, &rel_path) {
                Ok(text) => FileContent::Text(text),
                Err(_) => FileContent::Unreadable("Read failed".to_string()),
            }
        } else {
            FileContent::Unreadable("Special file".to_string())
        };

        files.push(FileContext {
            path: rel_path,
            content,
        });
    }

    Ok(ProjectContext {
        project,
        files,
    })
}

/// Coordinates the saving of a project back to the Project.md file.
pub fn save_project(root: &Path, project: &Project) -> Result<(), Error> {
    let content = serialize_project_md(project);
    write_file(root, Path::new("Project.md"), &content)
}

/// Selects a task from the project and transitions the agent to Executing.
pub fn select_task(agent: &mut Agent, project: &Project, task_id: &str) -> Result<(), Error> {
    // 1. Find the ProjectTask by task_id.
    let project_task = project
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or(Error::TaskNotFound)?;

    // 2. Reject if task is already Done.
    if project_task.status == TaskStatus::Done {
        return Err(Error::TaskAlreadyDone);
    }

    // 3. Construct the Agent's Task.
    let task = Task {
        id: project_task.id.clone(),
        description: project_task.description.clone(),
    };

    // 4. Transition the agent.
    agent.transition(Event::TaskSelected(task))
}

/// Records the successful completion of the current task.
///
/// Marks the task as Done in the project, persists it to disk, and
/// transitions the agent back to Planning.
pub fn complete_current_task(agent: &mut Agent, project: &mut Project) -> Result<(), Error> {
    // 1. Capture Agent's active task ID.
    let task_id = agent
        .task()
        .map(|t| t.id.clone())
        .ok_or(Error::NoActiveTask)?;

    // 2. Check Agent state to ensure the operation is valid for advance to Planning.
    if agent.state() != &State::Verifying {
        return Err(Error::InvalidStateTransition);
    }

    // 3. Find the matching ProjectTask in the caller's Project.
    if !project.tasks.iter().any(|t| t.id == task_id) {
        return Err(Error::TaskNotFound);
    }

    // 4. Clone the Project for a transactional update.
    let mut candidate = project.clone();

    // 5. Mark the matching task TaskStatus::Done in the clone.
    let task_in_candidate = candidate
        .tasks
        .iter_mut()
        .find(|t| t.id == task_id)
        .ok_or(Error::TaskNotFound)?;
    task_in_candidate.status = TaskStatus::Done;

    // 6. Persist the clone with save_project().
    let root = agent.root().ok_or(Error::Io)?;
    save_project(root, &candidate)?;

    // 7. If persistence succeeds: commit memory and transition agent.
    *project = candidate;
    agent.transition(Event::TestsPassed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::State;
    use tempfile::tempdir;
    use std::fs;

    #[test]
    fn test_open_valid_project() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let content = "My Project\n\nGoal\nBuild Magy\n\nTasks\n- [ ] Task 1";
        fs::write(root.join("Project.md"), content).unwrap();

        let (agent, project) = open_project(root.clone()).unwrap();

        assert_eq!(project.name, "My Project");
        assert_eq!(project.goal, "Build Magy");
        assert_eq!(agent.state(), &State::Planning);
        assert_eq!(agent.root(), Some(root.as_path()));
    }

    #[test]
    fn test_open_missing_project_file() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let result = open_project(root);
        assert_eq!(result, Err(Error::Io));
    }

    #[test]
    fn test_open_malformed_project() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        // Missing Tasks section
        let content = "My Project\n\nGoal\nBuild Magy";
        fs::write(root.join("Project.md"), content).unwrap();

        let result = open_project(root);
        assert_eq!(result, Err(Error::MissingTasks));
    }

    #[test]
    fn test_save_project() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let project = Project {
            name: "P".to_string(),
            goal: "G".to_string(),
            requirements: vec![],
            constraints: vec![],
            definition_of_done: vec![],
            tasks: vec![],
            current_status: "".to_string(),
        };

        save_project(&root, &project).unwrap();

        let content = fs::read_to_string(root.join("Project.md")).unwrap();
        let parsed = parse_project_md(&content).unwrap();
        assert_eq!(parsed, project);
    }

    #[test]
    fn test_save_project_io_error() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        // Create a directory named "Project.md".
        // This will cause the subsequent fs::write to fail with an IO error
        // while the path remains valid within the security boundary.
        fs::create_dir(root.join("Project.md")).unwrap();

        let project = Project {
            name: "P".to_string(),
            goal: "G".to_string(),
            requirements: vec![],
            constraints: vec![],
            definition_of_done: vec![],
            tasks: vec![],
            current_status: "".to_string(),
        };

        let result = save_project(&root, &project);
        assert_eq!(result, Err(Error::Io));
    }

    #[test]
    fn test_select_task_success() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let content = "P\n\nGoal\nG\n\nTasks\n- [ ] Task 1";
        fs::write(root.join("Project.md"), content).unwrap();

        let (mut agent, project) = open_project(root).unwrap();
        assert_eq!(agent.state(), &State::Planning);

        select_task(&mut agent, &project, "1").unwrap();

        assert_eq!(agent.state(), &State::Executing);
        assert_eq!(agent.task().unwrap().id, "1");
    }

    #[test]
    fn test_select_task_not_found() {
        let mut agent = Agent::new();
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();

        let project = Project {
            name: "P".to_string(),
            goal: "G".to_string(),
            requirements: vec![],
            constraints: vec![],
            definition_of_done: vec![],
            tasks: vec![],
            current_status: "".to_string(),
        };

        let result = select_task(&mut agent, &project, "99");
        assert_eq!(result, Err(Error::TaskNotFound));
        assert_eq!(agent.state(), &State::Planning);
    }

    #[test]
    fn test_select_task_already_done() {
        let mut agent = Agent::new();
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();

        let project = Project {
            name: "P".to_string(),
            goal: "G".to_string(),
            requirements: vec![],
            constraints: vec![],
            definition_of_done: vec![],
            tasks: vec![crate::domain::project::ProjectTask {
                id: "1".to_string(),
                description: "Done task".to_string(),
                status: TaskStatus::Done,
            }],
            current_status: "".to_string(),
        };

        let result = select_task(&mut agent, &project, "1");
        assert_eq!(result, Err(Error::TaskAlreadyDone));
        assert_eq!(agent.state(), &State::Planning);
    }

    #[test]
    fn test_select_task_invalid_state() {
        let mut agent = Agent::new();
        // Skip Start, try selecting in Idle
        let project = Project {
            name: "P".to_string(),
            goal: "G".to_string(),
            requirements: vec![],
            constraints: vec![],
            definition_of_done: vec![],
            tasks: vec![crate::domain::project::ProjectTask {
                id: "1".to_string(),
                description: "Open task".to_string(),
                status: TaskStatus::Open,
            }],
            current_status: "".to_string(),
        };

        let result = select_task(&mut agent, &project, "1");
        assert_eq!(result, Err(Error::InvalidStateTransition));
        assert_eq!(agent.state(), &State::Idle);
    }

    #[test]
    fn test_complete_task_success() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let content = "P\n\nGoal\nG\n\nTasks\n- [ ] T1";
        fs::write(root.join("Project.md"), content).unwrap();

        let (mut agent, mut project) = open_project(root).unwrap();
        select_task(&mut agent, &project, "1").unwrap();
        agent.transition(Event::ActionDone).unwrap();
        assert_eq!(agent.state(), &State::Verifying);

        complete_current_task(&mut agent, &mut project).unwrap();

        assert_eq!(agent.state(), &State::Planning);
        assert_eq!(agent.task(), None);
        assert_eq!(project.tasks[0].status, TaskStatus::Done);

        // Verify disk
        let disk_content = fs::read_to_string(agent.root().unwrap().join("Project.md")).unwrap();
        assert!(disk_content.contains("- [x] T1"));
    }

    #[test]
    fn test_complete_task_wrong_state() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let content = "P\n\nGoal\nG\n\nTasks\n- [ ] T1";
        fs::write(root.join("Project.md"), content).unwrap();

        let (mut agent, mut project) = open_project(root).unwrap();
        // Agent is in Planning, not Verifying

        select_task(&mut agent, &project, "1").unwrap();
        assert_eq!(agent.state(), &State::Executing);

        let result = complete_current_task(&mut agent, &mut project);
        assert_eq!(result, Err(Error::InvalidStateTransition));
        assert_eq!(agent.state(), &State::Executing);
        assert_eq!(project.tasks[0].status, TaskStatus::Open);
    }

    #[test]
    fn test_complete_task_missing_active() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let content = "P\n\nGoal\nG\n\nTasks\n- [ ] T1";
        fs::write(root.join("Project.md"), content).unwrap();

        let (mut agent, mut project) = open_project(root).unwrap();

        let result = complete_current_task(&mut agent, &mut project);
        assert_eq!(result, Err(Error::NoActiveTask));
    }

    #[test]
    fn test_complete_task_missing_in_project() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let content = "P\n\nGoal\nG\n\nTasks\n- [ ] T1";
        fs::write(root.join("Project.md"), content).unwrap();

        let (mut agent, mut project) = open_project(root).unwrap();

        select_task(&mut agent, &project, "1").unwrap();
        agent.transition(Event::ActionDone).unwrap();

        // Remove task from project manually
        project.tasks.clear();

        let result = complete_current_task(&mut agent, &mut project);
        assert_eq!(result, Err(Error::TaskNotFound));
        assert_eq!(agent.state(), &State::Verifying);
    }

    #[test]
    fn test_complete_task_persistence_failure() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let content = "P\n\nGoal\nG\n\nTasks\n- [ ] T1";
        fs::write(root.join("Project.md"), content).unwrap();

        let (mut agent, mut project) = open_project(root).unwrap();
        select_task(&mut agent, &project, "1").unwrap();
        agent.transition(Event::ActionDone).unwrap();

        // Cause persistence failure by creating a directory where Project.md should be
        fs::remove_file(agent.root().unwrap().join("Project.md")).unwrap();
        fs::create_dir(agent.root().unwrap().join("Project.md")).unwrap();

        let result = complete_current_task(&mut agent, &mut project);
        assert_eq!(result, Err(Error::Io));

        // Agent and Project must remain unchanged
        assert_eq!(agent.state(), &State::Verifying);
        assert_eq!(project.tasks[0].status, TaskStatus::Open);
    }

    #[test]
    fn test_assemble_project_context() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        // Project.md
        let content = "My Project\n\nGoal\nTest\n\nTasks\n- [ ] T1";
        fs::write(root.join("Project.md"), content).unwrap();

        // Subdir and files
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn main() {}").unwrap();
        fs::create_dir(root.join("empty")).unwrap();

        let (_, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();

        assert_eq!(context.project, project);

        // discover_files returns: Project.md, empty, src, src/lib.rs
        assert_eq!(context.files.len(), 4);

        let p_md = &context.files[0];
        assert_eq!(p_md.path, PathBuf::from("Project.md"));
        if let FileContent::Text(ref t) = p_md.content {
            assert!(t.contains("My Project"));
        } else {
            panic!("Project.md should be text");
        }

        assert_eq!(context.files[1].path, PathBuf::from("empty"));
        assert_eq!(context.files[1].content, FileContent::Directory);

        assert_eq!(context.files[2].path, PathBuf::from("src"));
        assert_eq!(context.files[2].content, FileContent::Directory);

        assert_eq!(context.files[3].path, PathBuf::from("src/lib.rs"));
        assert_eq!(context.files[3].content, FileContent::Text("fn main() {}".to_string()));
    }

    #[test]
    fn test_assemble_project_context_unreadable() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        // Binary file (invalid UTF-8)
        fs::write(root.join("binary.bin"), vec![0, 159, 146, 150]).unwrap();

        let (_, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root, project).unwrap();

        let bin_entry = context.files.iter().find(|f| f.path == PathBuf::from("binary.bin")).unwrap();
        if let FileContent::Unreadable(ref reason) = bin_entry.content {
            assert_eq!(reason, "Read failed");
        } else {
            panic!("Binary file should be unreadable");
        }
    }

    #[test]
    fn test_plan_execution() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let content = "P\n\nGoal\nG\n\nTasks\n- [x] T1\n- [ ] T2\n- [ ] T3";
        fs::write(root.join("Project.md"), content).unwrap();

        let (agent, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root, project).unwrap();

        let plan = plan_execution(&agent, &context).unwrap();

        // T1 is excluded (Done), T2 and T3 are included (Open)
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].id, "2");
        assert_eq!(plan.tasks[0].description, "T2");
        assert_eq!(plan.tasks[1].id, "3");
        assert_eq!(plan.tasks[1].description, "T3");
    }

    #[test]
    fn test_plan_execution_invalid_state() {
        let agent = Agent::new(); // Idle state
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

        let result = plan_execution(&agent, &context);
        assert_eq!(result, Err(Error::InvalidStateTransition));
    }

    #[test]
    fn test_execute_tool_read_file() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();
        fs::write(root.join("hello.txt"), "world").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        select_task(&mut agent, &project, "1").unwrap();
        assert_eq!(agent.state(), &State::Executing);

        let request = ToolRequest::ReadFile { path: PathBuf::from("hello.txt") };
        let result = execute_tool(&agent, request);

        assert_eq!(result, ToolResult::Text("world".to_string()));
    }

    #[test]
    fn test_execute_tool_write_file() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let request = ToolRequest::WriteFile {
            path: PathBuf::from("new.txt"),
            content: "new content".to_string()
        };
        let result = execute_tool(&agent, request);

        assert_eq!(result, ToolResult::Success);
        assert_eq!(fs::read_to_string(root.join("new.txt")).unwrap(), "new content");
    }

    #[test]
    fn test_execute_tool_list_directory() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();
        fs::create_dir(root.join("src")).unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let request = ToolRequest::ListDirectory { path: PathBuf::from(".") };
        let result = execute_tool(&agent, request);

        if let ToolResult::Entries(entries) = result {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].name, "Project.md");
            assert_eq!(entries[1].name, "src");
        } else {
            panic!("Expected Entries result");
        }
    }

    #[test]
    fn test_execute_tool_discover_files() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let request = ToolRequest::DiscoverFiles;
        let result = execute_tool(&agent, request);

        if let ToolResult::Paths(paths) = result {
            assert_eq!(paths.len(), 1);
            assert_eq!(paths[0], PathBuf::from("Project.md"));
        } else {
            panic!("Expected Paths result");
        }
    }

    #[test]
    fn test_execute_tool_run_command() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let request = ToolRequest::RunCommand { command: "echo executed".to_string() };
        let result = execute_tool(&agent, request);

        if let ToolResult::Command(out) = result {
            assert_eq!(out.stdout.trim(), "executed");
            assert_eq!(out.exit_code, Some(0));
        } else {
            panic!("Expected Command result");
        }
    }

    #[test]
    fn test_execute_tool_invalid_state() {
        let agent = Agent::new();
        let request = ToolRequest::DiscoverFiles;
        let result = execute_tool(&agent, request);
        assert_eq!(result, ToolResult::Error("Agent is not in Executing state".to_string()));
    }

    struct MockProvider {
        response: Result<ModelResponse, Error>,
    }
    impl ModelProvider for MockProvider {
        fn ask(&self, _req: ModelRequest) -> Result<ModelResponse, Error> {
            self.response.clone()
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

        let result = run_reasoning_step(&agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "prompt").unwrap();

        let record = result.action_record.unwrap();
        assert_eq!(record.request, ToolRequest::ReadFile { path: PathBuf::from("test.txt") });
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

        let result = run_reasoning_step(&agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "prompt").unwrap();

        let record = result.action_record.unwrap();
        assert_eq!(record.approval_status, ApprovalStatus::Pending);
        assert_eq!(record.outcome, ExecutionOutcome::AwaitingApproval);
        assert!(!root.join("out.txt").exists());
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

        let result = run_reasoning_step(&agent, &context, &plan, &[], &provider, &policy, "prompt").unwrap();

        let record = result.action_record.unwrap();
        assert_eq!(record.approval_status, ApprovalStatus::Denied);
        assert_eq!(record.outcome, ExecutionOutcome::Denied);
    }

    #[test]
    fn test_run_reasoning_step_invalid_state() {
        let agent = Agent::new(); // Idle state
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

        let result = run_reasoning_step(&agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "S");
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

        let provider = MockProvider {
            response: Err(Error::ModelError("Bad request".to_string())),
        };

        let result = run_reasoning_step(&agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "S");
        assert_eq!(result, Err(Error::ModelError("Bad request".to_string())));
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
            response: Ok(ModelResponse {
                content: "```json\n{\"tool\": \"non_existent\"}\n```".to_string(),
            }),
        };

        let result = run_reasoning_step(&agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "S").unwrap();
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
            response: Ok(ModelResponse {
                content: "```json\n{\"tool\": \"read_file\", \"path\": \"../escape.txt\"}\n```".to_string(),
            }),
        };

        let result = run_reasoning_step(&agent, &context, &plan, &[], &provider, &DefaultApprovalPolicy, "S").unwrap();
        let record = result.action_record.unwrap();
        assert!(matches!(record.outcome, ExecutionOutcome::Executed(ToolResult::Error(_))));
    }

    struct MultiMockProvider {
        responses: std::cell::RefCell<Vec<Result<ModelResponse, Error>>>,
    }
    impl ModelProvider for MultiMockProvider {
        fn ask(&self, _req: ModelRequest) -> Result<ModelResponse, Error> {
            self.responses.borrow_mut().remove(0)
        }
    }

    #[test]
    fn test_run_execution_cycle_sequential() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();
        fs::write(root.join("input.txt"), "data").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse { content: "```json\n{\"tool\": \"read_file\", \"path\": \"input.txt\"}\n```".to_string() }),
                Ok(ModelResponse { content: "Done.".to_string() }),
            ]),
        };

        let trace = run_execution_cycle(&agent, &context, &plan, &provider, &DefaultApprovalPolicy, "S", 5).unwrap();

        assert_eq!(trace.steps.len(), 2);
        assert_eq!(trace.stopped_reason, "Model stopped without action");
        assert_eq!(trace.steps[0].action_record.as_ref().unwrap().outcome, ExecutionOutcome::Executed(ToolResult::Text("data".to_string())));
    }

    #[test]
    fn test_run_execution_cycle_pending_stop() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root.clone(), project.clone()).unwrap();
        let plan = plan_execution(&agent, &context).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        let provider = MultiMockProvider {
            responses: std::cell::RefCell::new(vec![
                Ok(ModelResponse { content: "```json\n{\"tool\": \"write_file\", \"path\": \"out.txt\", \"content\": \"data\"}\n```".to_string() }),
            ]),
        };

        let trace = run_execution_cycle(&agent, &context, &plan, &provider, &DefaultApprovalPolicy, "S", 5).unwrap();

        assert_eq!(trace.steps.len(), 1);
        assert_eq!(trace.stopped_reason, "Action requires approval");
        assert_eq!(trace.steps[0].action_record.as_ref().unwrap().outcome, ExecutionOutcome::AwaitingApproval);
    }
}
