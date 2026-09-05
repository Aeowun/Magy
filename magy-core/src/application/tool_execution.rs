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
use crate::domain::tool::{ToolRequest, ToolResult};
use crate::infrastructure::filesystem::{read_file, write_file, list_directory, discover_files};
use crate::infrastructure::command::run_project_command;

/// Executes a tool request within the project boundary.
pub fn execute_tool(agent: &Agent, request: ToolRequest) -> ToolResult {
    if agent.state() != &State::Executing {
        return ToolResult::Error("Agent is not in Executing state".to_string());
    }

    let root = match agent.root() {
        Some(r) => r,
        None => return ToolResult::Error("Agent has no project root".to_string()),
    };

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
        ToolRequest::TaskComplete => ToolResult::Error("TaskComplete is a control signal and cannot be executed as a tool".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::fs;
    use tempfile::tempdir;
    use crate::application::project_lifecycle::open_project;
    use crate::application::task_lifecycle::select_task;

    #[test]
    fn test_execute_tool_read_file() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();
        fs::write(root.join("hello.txt"), "world").unwrap();

        let (mut agent, project) = open_project(root.clone()).unwrap();
        select_task(&mut agent, &project, "1").unwrap();

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
            assert!(entries.iter().any(|e| e.name == "Project.md"));
            assert!(entries.iter().any(|e| e.name == "src"));
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
            assert!(paths.contains(&PathBuf::from("Project.md")));
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
}
