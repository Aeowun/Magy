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
use serde::Deserialize;
use serde_json::json;
use crate::Error;
use crate::domain::agent::{Agent, Event};
use crate::domain::project::{Project, ProjectTask, TaskStatus, ProjectContext, parse_project_md, serialize_project_md};
use crate::domain::model::{ModelProvider, ModelRequest};
use crate::infrastructure::filesystem::{read_file, write_file, discover_files};

/// Coordinates the opening of a project: reading the file, parsing it,
/// and initializing the agent run.
pub fn open_project(root: PathBuf) -> Result<(Agent, Project), Error> {
    let content = read_file(&root, Path::new("Project.md"))?;
    let project = parse_project_md(&content)?;
    let mut agent = Agent::new();
    agent.transition(Event::Start(root))?;
    Ok((agent, project))
}

/// Coordinates the saving of a project back to the Project.md file.
pub fn save_project(root: &Path, project: &Project) -> Result<(), Error> {
    let content = serialize_project_md(project);
    write_file(root, Path::new("Project.md"), &content)
}

#[derive(Deserialize)]
struct GeneratedProject {
    name: String,
    goal: String,
    requirements: Vec<String>,
    constraints: Vec<String>,
    definition_of_done: Vec<String>,
    tasks: Vec<String>,
}

/// Generates a Project.md file from a user-supplied goal if one is missing.
pub fn initialize_project(
    root: PathBuf,
    provider: &dyn ModelProvider,
    user_goal: &str,
) -> Result<Project, Error> {
    // 1. Check if Project.md already exists.
    match read_file(&root, Path::new("Project.md")) {
        Ok(content) => {
            // Exists and is readable. Load it.
            return parse_project_md(&content);
        }
        Err(Error::FileNotFound) => {
            // Specifically missing. Proceed to generation.
        }
        Err(e) => {
            // Other errors (e.g. unreadable, permission denied, outside boundary).
            return Err(e);
        }
    }

    // 2. Prepare the generation request.
    let system_prompt = "You are a software architect. Given a high-level goal, generate a detailed project specification for an AI agent.
Provide a clear project name, refine the goal, identify key requirements and constraints, define what success looks like (Definition of Done), and break the work down into a sequence of actionable tasks.
Output ONLY a JSON object matching the requested schema.";

    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "goal": { "type": "string" },
            "requirements": { "type": "array", "items": { "type": "string" } },
            "constraints": { "type": "array", "items": { "type": "string" } },
            "definition_of_done": { "type": "array", "items": { "type": "string" } },
            "tasks": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["name", "goal", "requirements", "constraints", "definition_of_done", "tasks"],
        "additionalProperties": false
    });

    // Discovery files for context (even if minimal)
    let paths = discover_files(&root).unwrap_or_default();
    let files = paths.into_iter().map(|p| crate::domain::project::FileContext {
        path: p,
        content: crate::domain::project::FileContent::Unreadable("Initializing".to_string()),
    }).collect();

    let dummy_project = Project {
        name: "New Project".to_string(),
        goal: user_goal.to_string(),
        requirements: vec![],
        constraints: vec![],
        definition_of_done: vec![],
        tasks: vec![],
        current_status: "Initializing".to_string(),
    };

    let request = ModelRequest {
        system_prompt: system_prompt.to_string(),
        context: ProjectContext { project: dummy_project, files },
        task: None,
        plan: None,
        history: vec![],
        schema: Some(schema),
    };

    // 3. Ask the model.
    let response = provider.ask(request)?;

    // 4. Parse the generated JSON.
    let generated: GeneratedProject = serde_json::from_str(&response.content)
        .map_err(|e| Error::ModelError(format!("Failed to parse generated project: {}", e)))?;

    // 5. Convert to Domain model.
    let project = Project {
        name: generated.name,
        goal: generated.goal,
        requirements: generated.requirements,
        constraints: generated.constraints,
        definition_of_done: generated.definition_of_done,
        tasks: generated.tasks.into_iter().enumerate().map(|(i, desc)| ProjectTask {
            id: (i + 1).to_string(),
            description: desc,
            status: TaskStatus::Open,
        }).collect(),
        current_status: "Initialized".to_string(),
    };

    // 6. Save to disk.
    save_project(&root, &project)?;

    Ok(project)
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
        assert_eq!(result, Err(Error::FileNotFound));
    }

    #[test]
    fn test_open_malformed_project() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
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

    struct MockProvider {
        response: String,
    }
    impl ModelProvider for MockProvider {
        fn ask(&self, _req: ModelRequest) -> Result<crate::domain::model::ModelResponse, Error> {
            Ok(crate::domain::model::ModelResponse { content: self.response.clone() })
        }
    }

    #[test]
    fn test_initialize_project_success() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let json_resp = json!({
            "name": "Test App",
            "goal": "Build a test app",
            "requirements": ["Req 1"],
            "constraints": ["Cons 1"],
            "definition_of_done": ["DoD 1"],
            "tasks": ["Task 1", "Task 2"]
        }).to_string();

        let provider = MockProvider { response: json_resp };
        let project = initialize_project(root.clone(), &provider, "My goal").unwrap();

        assert_eq!(project.name, "Test App");
        assert_eq!(project.tasks.len(), 2);
        assert_eq!(project.tasks[0].description, "Task 1");
        assert_eq!(project.tasks[0].id, "1");

        // Verify file was saved
        let content = fs::read_to_string(root.join("Project.md")).unwrap();
        assert!(content.contains("Test App"));
        assert!(content.contains("- [ ] Task 1"));
    }

    #[test]
    fn test_initialize_project_existing_file() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let existing = "Old Name\n\nGoal\nOld Goal\n\nTasks\n- [ ] Old Task";
        fs::write(root.join("Project.md"), existing).unwrap();

        let provider = MockProvider { response: "{}".to_string() }; // Should not be called
        let project = initialize_project(root, &provider, "New Goal").unwrap();

        assert_eq!(project.name, "Old Name");
        assert_eq!(project.tasks[0].description, "Old Task");
    }

    #[test]
    fn test_initialize_project_llm_failure() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let provider = MockProvider { response: "not json".to_string() };
        let result = initialize_project(root.clone(), &provider, "goal");

        assert!(matches!(result, Err(Error::ModelError(_))));
        assert!(!root.join("Project.md").exists());
    }
}
