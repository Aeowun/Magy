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

use std::path::PathBuf;
use crate::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Open,
    Done,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectTask {
    pub id: String,
    pub description: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub name: String,
    pub goal: String,
    pub requirements: Vec<String>,
    pub constraints: Vec<String>,
    pub definition_of_done: Vec<String>,
    pub tasks: Vec<ProjectTask>,
    pub current_status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileContent {
    Text(String),
    Directory,
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileContext {
    pub path: PathBuf,
    pub content: FileContent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectContext {
    pub project: Project,
    pub files: Vec<FileContext>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectPlan {
    pub tasks: Vec<ProjectTask>,
}

#[derive(Debug, PartialEq)]
enum ParserSection {
    None,
    Goal,
    Requirements,
    Constraints,
    DefinitionOfDone,
    Tasks,
    CurrentStatus,
}

pub fn parse_project_md(content: &str) -> Result<Project, Error> {
    let mut name = String::new();
    let mut goal_lines = Vec::new();
    let mut requirements = Vec::new();
    let mut constraints = Vec::new();
    let mut dod = Vec::new();
    let mut tasks = Vec::new();
    let mut status_lines = Vec::new();

    let mut current_section = ParserSection::None;
    let mut sections_found = std::collections::HashSet::new();

    for line in content.lines() {
        let trimmed = line.trim();

        if name.is_empty() && !trimmed.is_empty() {
            name = trimmed.to_string();
            continue;
        }

        let section = match trimmed {
            "Goal" => Some(ParserSection::Goal),
            "Requirements" => Some(ParserSection::Requirements),
            "Constraints" => Some(ParserSection::Constraints),
            "Definition of Done" => Some(ParserSection::DefinitionOfDone),
            "Tasks" => Some(ParserSection::Tasks),
            "Current Status" => Some(ParserSection::CurrentStatus),
            _ => None,
        };

        if let Some(s) = section {
            current_section = s;
            sections_found.insert(trimmed.to_string());
            continue;
        }

        match current_section {
            ParserSection::Goal => goal_lines.push(line),
            ParserSection::CurrentStatus => status_lines.push(line),
            ParserSection::Requirements => {
                if trimmed.starts_with("- ") {
                    requirements.push(trimmed[2..].to_string());
                }
            }
            ParserSection::Constraints => {
                if trimmed.starts_with("- ") {
                    constraints.push(trimmed[2..].to_string());
                }
            }
            ParserSection::DefinitionOfDone => {
                if trimmed.starts_with("- ") {
                    dod.push(trimmed[2..].to_string());
                }
            }
            ParserSection::Tasks => {
                let status = if trimmed.starts_with("- [ ] ") {
                    Some(TaskStatus::Open)
                } else if trimmed.starts_with("- [x] ") || trimmed.starts_with("- [X] ") {
                    Some(TaskStatus::Done)
                } else {
                    None
                };

                if let Some(s) = status {
                    let desc = trimmed[6..].to_string();
                    tasks.push(ProjectTask {
                        id: (tasks.len() + 1).to_string(),
                        description: desc,
                        status: s,
                    });
                }
            }
            ParserSection::None => {}
        }
    }

    if !sections_found.contains("Goal") {
        return Err(Error::MissingGoal);
    }
    if !sections_found.contains("Tasks") {
        return Err(Error::MissingTasks);
    }

    Ok(Project {
        name,
        goal: goal_lines.join("\n").trim().to_string(),
        requirements,
        constraints,
        definition_of_done: dod,
        tasks,
        current_status: status_lines.join("\n").trim().to_string(),
    })
}

pub fn serialize_project_md(project: &Project) -> String {
    let mut out = String::new();

    // Project Name
    out.push_str(&project.name);
    out.push_str("\n\n");

    // Goal (Required)
    out.push_str("Goal\n");
    out.push_str(&project.goal);
    out.push_str("\n\n");

    // Requirements (Optional)
    if !project.requirements.is_empty() {
        out.push_str("Requirements\n");
        for req in &project.requirements {
            out.push_str("- ");
            out.push_str(req);
            out.push('\n');
        }
        out.push('\n');
    }

    // Constraints (Optional)
    if !project.constraints.is_empty() {
        out.push_str("Constraints\n");
        for c in &project.constraints {
            out.push_str("- ");
            out.push_str(c);
            out.push('\n');
        }
        out.push('\n');
    }

    // Definition of Done (Optional)
    if !project.definition_of_done.is_empty() {
        out.push_str("Definition of Done\n");
        for dod in &project.definition_of_done {
            out.push_str("- ");
            out.push_str(dod);
            out.push('\n');
        }
        out.push('\n');
    }

    // Tasks (Required)
    out.push_str("Tasks\n");
    for t in &project.tasks {
        let marker = match t.status {
            TaskStatus::Open => "[ ]",
            TaskStatus::Done => "[x]",
        };
        out.push_str("- ");
        out.push_str(marker);
        out.push(' ');
        out.push_str(&t.description);
        out.push('\n');
    }
    out.push('\n');

    // Current Status (Optional in model, but part of spec)
    if !project.current_status.is_empty() {
        out.push_str("Current Status\n");
        out.push_str(&project.current_status);
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_canonical() {
        let content = "My Project

Goal
Build a website.

Requirements
- React
- Rust

Constraints
- No APIs

Definition of Done
- Tests pass

Tasks
- [x] Task 1
- [ ] Task 2

Current Status
In progress";

        let project = parse_project_md(content).unwrap();
        assert_eq!(project.name, "My Project");
        assert_eq!(project.goal, "Build a website.");
        assert_eq!(project.requirements, vec!["React", "Rust"]);
        assert_eq!(project.constraints, vec!["No APIs"]);
        assert_eq!(project.definition_of_done, vec!["Tests pass"]);
        assert_eq!(project.tasks.len(), 2);
        assert_eq!(project.tasks[0].status, TaskStatus::Done);
        assert_eq!(project.tasks[1].status, TaskStatus::Open);
        assert_eq!(project.current_status, "In progress");
    }

    #[test]
    fn test_parse_minimal_valid() {
        let content = "Mini\n\nGoal\nWin\n\nTasks\n- [ ] Do it";
        let project = parse_project_md(content).unwrap();
        assert_eq!(project.name, "Mini");
        assert_eq!(project.goal, "Win");
        assert_eq!(project.tasks.len(), 1);
    }

    #[test]
    fn test_parse_missing_required() {
        assert_eq!(parse_project_md("Name\n\nTasks\n- [ ] T"), Err(Error::MissingGoal));
        assert_eq!(parse_project_md("Name\n\nGoal\nG"), Err(Error::MissingTasks));
    }

    #[test]
    fn test_parse_id_sequential() {
        let content = "P\n\nGoal\nG\n\nTasks\n- [ ] A\n- [ ] B";
        let project = parse_project_md(content).unwrap();
        assert_eq!(project.tasks[0].id, "1");
        assert_eq!(project.tasks[1].id, "2");
    }

    #[test]
    fn test_parse_plain_text_multiline() {
        let content = "P\n\nGoal\nLine 1\nLine 2\n\nTasks\n- [ ] T";
        let project = parse_project_md(content).unwrap();
        assert_eq!(project.goal, "Line 1\nLine 2");
    }

    #[test]
    fn test_parse_status_variants() {
        let content = "P\n\nGoal\nG\n\nTasks\n- [x] a\n- [X] b";
        let project = parse_project_md(content).unwrap();
        assert_eq!(project.tasks[0].status, TaskStatus::Done);
        assert_eq!(project.tasks[1].status, TaskStatus::Done);
    }

    #[test]
    fn test_serialize_canonical() {
        let project = Project {
            name: "Magy".to_string(),
            goal: "Goal line 1\nGoal line 2".to_string(),
            requirements: vec!["Req 1".to_string()],
            constraints: vec!["Cons 1".to_string()],
            definition_of_done: vec!["DoD 1".to_string()],
            tasks: vec![
                ProjectTask { id: "1".to_string(), description: "T1".to_string(), status: TaskStatus::Done },
                ProjectTask { id: "2".to_string(), description: "T2".to_string(), status: TaskStatus::Open },
            ],
            current_status: "Status line 1\nStatus line 2".to_string(),
        };

        let output = serialize_project_md(&project);

        let expected = "Magy\n\nGoal\nGoal line 1\nGoal line 2\n\nRequirements\n- Req 1\n\nConstraints\n- Cons 1\n\nDefinition of Done\n- DoD 1\n\nTasks\n- [x] T1\n- [ ] T2\n\nCurrent Status\nStatus line 1\nStatus line 2\n";

        assert_eq!(output, expected);
    }

    #[test]
    fn test_serialize_parse_roundtrip() {
        let project = Project {
            name: "Roundtrip".to_string(),
            goal: "Multi-line\nGoal description".to_string(),
            requirements: vec!["R1".to_string(), "R2".to_string()],
            constraints: vec!["C1".to_string()],
            definition_of_done: vec!["D1".to_string()],
            tasks: vec![
                ProjectTask { id: "1".to_string(), description: "Task 1".to_string(), status: TaskStatus::Done },
                ProjectTask { id: "2".to_string(), description: "Task 2".to_string(), status: TaskStatus::Open },
            ],
            current_status: "Working".to_string(),
        };

        let serialized = serialize_project_md(&project);
        let parsed = parse_project_md(&serialized).unwrap();

        assert_eq!(parsed, project);
    }
}
