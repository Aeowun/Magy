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

use crate::domain::agent::{Agent, State};
use crate::domain::model::{ModelProvider, PlannerRequest, PlannerResponse};
use crate::domain::project::{Project, ProjectContext, ProjectPlan, TaskStatus};
use crate::Error;
use std::collections::HashSet;

/// Produces a deterministic execution plan from the project context.
pub fn plan_execution(agent: &Agent, context: &ProjectContext) -> Result<ProjectPlan, Error> {
    if agent.state() != &State::Planning {
        return Err(Error::InvalidStateTransition);
    }

    let open_tasks: Vec<_> = context
        .project
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Open)
        .cloned()
        .collect();

    Ok(ProjectPlan { tasks: open_tasks })
}

/// Orchestrates an LLM-backed project planning session.
pub fn plan_project(
    project: &Project,
    context: &ProjectContext,
    provider: &dyn ModelProvider,
    system_prompt: &str,
) -> Result<PlannerResponse, Error> {
    let request = PlannerRequest {
        system_prompt: system_prompt.to_string(),
        goal: project.goal.clone(),
        requirements: project.requirements.clone(),
        constraints: project.constraints.clone(),
        definition_of_done: project.definition_of_done.clone(),
        existing_tasks: project.tasks.clone(),
        context: context.clone(),
    };

    let response = provider.plan(request)?;
    validate_plan(&response, project)?;

    Ok(response)
}

fn validate_plan(response: &PlannerResponse, _project: &Project) -> Result<(), Error> {
    if response.tasks.is_empty() {
        return Err(Error::ParseError("Planner returned an empty task list".to_string()));
    }

    let mut ids = HashSet::new();
    for task in &response.tasks {
        if task.id.is_empty() {
            return Err(Error::ParseError("Task ID cannot be empty".to_string()));
        }
        if !ids.insert(task.id.clone()) {
            return Err(Error::ParseError(format!("Duplicate task ID: {}", task.id)));
        }
        if task.description.is_empty() {
            return Err(Error::ParseError(format!("Task {} description cannot be empty", task.id)));
        }
        if task.acceptance_criteria.is_empty() {
            return Err(Error::ParseError(format!(
                "Task {} must have at least one acceptance criterion",
                task.id
            )));
        }
        if task.status != TaskStatus::Open {
             return Err(Error::ParseError(format!(
                "Planner attempted to claim task {} is already complete",
                task.id
            )));
        }
    }

    // Dependency validation (Placeholder - assuming simple linear for now, but contract allows for DAG)
    // In a full implementation, we'd check for cycles and valid references here.

    if response.tasks.len() > 50 {
         return Err(Error::ParseError("Planner returned too many tasks (>50)".to_string()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::context_assembly::assemble_project_context;
    use crate::application::project_lifecycle::open_project;
    use crate::domain::model::ModelRequest;
    use crate::domain::project::Project;
    use std::fs;
    use tempfile::tempdir;

    struct MockPlanner {
        response: Result<PlannerResponse, Error>,
    }
    impl ModelProvider for MockPlanner {
        fn ask(&self, _: ModelRequest) -> Result<crate::domain::model::ModelResponse, Error> {
            unreachable!()
        }
        fn plan(&self, _: PlannerRequest) -> Result<PlannerResponse, Error> {
            self.response.clone()
        }
    }

    #[test]
    fn test_plan_project_validation_empty() {
        let project = Project {
            name: "P".to_string(),
            goal: "G".to_string(),
            requirements: vec![],
            constraints: vec![],
            definition_of_done: vec![],
            tasks: vec![],
            current_status: "".to_string(),
            plan_version: 0,
            plan_created_at_ms: None,
            replan_count: 0,
            replan_reason: None,
        };
        let context = ProjectContext { project: project.clone(), files: vec![] };
        let provider = MockPlanner {
            response: Ok(PlannerResponse { plan_version: 1, tasks: vec![] }),
        };

        let res = plan_project(&project, &context, &provider, "");
        assert!(matches!(res, Err(Error::ParseError(msg)) if msg.contains("empty")));
    }

    #[test]
    fn test_plan_project_validation_duplicates() {
        let project = Project {
            name: "P".to_string(),
            goal: "G".to_string(),
            requirements: vec![],
            constraints: vec![],
            definition_of_done: vec![],
            tasks: vec![],
            current_status: "".to_string(),
            plan_version: 0,
            plan_created_at_ms: None,
            replan_count: 0,
            replan_reason: None,
        };
        let context = ProjectContext { project: project.clone(), files: vec![] };
        let task = crate::domain::project::ProjectTask {
            id: "1".to_string(),
            description: "D".to_string(),
            status: TaskStatus::Open,
            acceptance_criteria: vec!["C".to_string()],
            evidence: vec![],
        };
        let provider = MockPlanner {
            response: Ok(PlannerResponse { plan_version: 1, tasks: vec![task.clone(), task] }),
        };

        let res = plan_project(&project, &context, &provider, "");
        assert!(matches!(res, Err(Error::ParseError(msg)) if msg.contains("Duplicate")));
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

        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].id, "2");
        assert_eq!(plan.tasks[1].id, "3");
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
                plan_version: 0,
                plan_created_at_ms: None,
                replan_count: 0,
                replan_reason: None,
            },
            files: vec![],
        };

        let result = plan_execution(&agent, &context);
        assert_eq!(result, Err(Error::InvalidStateTransition));
    }
}
