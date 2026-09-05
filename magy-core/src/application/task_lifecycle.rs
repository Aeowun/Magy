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

use crate::application::project_lifecycle::save_project;
use crate::domain::agent::{Agent, Event, State, Task};
use crate::domain::project::{Project, TaskStatus};
use crate::Error;

/// Selects a task from the project and transitions the agent to Executing.
pub fn select_task(agent: &mut Agent, project: &Project, task_id: &str) -> Result<(), Error> {
    let project_task = project
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or(Error::TaskNotFound)?;

    if project_task.status == TaskStatus::Done {
        return Err(Error::TaskAlreadyDone);
    }

    let task = Task {
        id: project_task.id.clone(),
        description: project_task.description.clone(),
    };

    agent.transition(Event::TaskSelected(task))
}

/// Records the successful completion of the current task.
pub fn complete_current_task(agent: &mut Agent, project: &mut Project) -> Result<(), Error> {
    let task_id = agent
        .task()
        .map(|t| t.id.clone())
        .ok_or(Error::NoActiveTask)?;

    if agent.state() != &State::Verifying {
        return Err(Error::InvalidStateTransition);
    }

    if !project.tasks.iter().any(|t| t.id == task_id) {
        return Err(Error::TaskNotFound);
    }

    let mut candidate = project.clone();

    let task_in_candidate = candidate
        .tasks
        .iter_mut()
        .find(|t| t.id == task_id)
        .ok_or(Error::TaskNotFound)?;
    task_in_candidate.status = TaskStatus::Done;

    let root = agent.root().ok_or(Error::Io)?;
    save_project(root, &candidate)?;

    *project = candidate;
    agent.transition(Event::TestsPassed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::project_lifecycle::open_project;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_select_task_success() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let content = "P\n\nGoal\nG\n\nTasks\n- [ ] Task 1";
        fs::write(root.join("Project.md"), content).unwrap();

        let (mut agent, project) = open_project(root).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

        assert_eq!(agent.state(), &State::Executing);
        assert_eq!(agent.task().unwrap().id, "1");
    }

    #[test]
    fn test_select_task_not_found() {
        let mut agent = Agent::new();
        agent
            .transition(Event::Start(PathBuf::from("/test")))
            .unwrap();

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
    }

    #[test]
    fn test_select_task_already_done() {
        let mut agent = Agent::new();
        agent
            .transition(Event::Start(PathBuf::from("/test")))
            .unwrap();

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
    }

    #[test]
    fn test_select_task_invalid_state() {
        let mut agent = Agent::new();
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
        complete_current_task(&mut agent, &mut project).unwrap();

        assert_eq!(agent.state(), &State::Planning);
        assert_eq!(project.tasks[0].status, TaskStatus::Done);
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
        select_task(&mut agent, &project, "1").unwrap();
        let result = complete_current_task(&mut agent, &mut project);
        assert_eq!(result, Err(Error::InvalidStateTransition));
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
        project.tasks.clear();

        let result = complete_current_task(&mut agent, &mut project);
        assert_eq!(result, Err(Error::TaskNotFound));
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

        fs::remove_file(agent.root().unwrap().join("Project.md")).unwrap();
        fs::create_dir(agent.root().unwrap().join("Project.md")).unwrap();

        let result = complete_current_task(&mut agent, &mut project);
        assert_eq!(result, Err(Error::Io));
    }
}
