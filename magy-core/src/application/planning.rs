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
use crate::domain::agent::{Agent, State};
use crate::domain::project::{ProjectContext, ProjectPlan, TaskStatus};

/// Produces a deterministic execution plan from the project context.
pub fn plan_execution(agent: &Agent, context: &ProjectContext) -> Result<ProjectPlan, Error> {
    if agent.state() != &State::Planning {
        return Err(Error::InvalidStateTransition);
    }

    let open_tasks: Vec<_> = context.project.tasks.iter()
        .filter(|t| t.status == TaskStatus::Open)
        .cloned()
        .collect();

    Ok(ProjectPlan {
        tasks: open_tasks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use crate::domain::project::Project;
    use crate::application::project_lifecycle::open_project;
    use crate::application::context_assembly::assemble_project_context;

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
            },
            files: vec![],
        };

        let result = plan_execution(&agent, &context);
        assert_eq!(result, Err(Error::InvalidStateTransition));
    }
}
