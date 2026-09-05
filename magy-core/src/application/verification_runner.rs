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
use crate::domain::model::VerificationResult;
use crate::infrastructure::command::run_project_command;
use crate::Error;

/// Selects a conservative verification command from the project files.
///
/// An empty result means the project has no recognized test runner. Such a
/// project cannot be declared verified by the generic runner.
pub fn recommended_verification_command(root: &std::path::Path) -> Option<String> {
    if let Some(command) = project_verification_command(root) {
        return Some(command);
    }
    return if root.join("Cargo.toml").is_file() {
        Some("cargo test".to_string())
    } else if root.join("package.json").is_file() {
        Some("npm test".to_string())
    } else if root.join("pyproject.toml").is_file()
        || root.join("pytest.ini").is_file()
        || root.join("requirements.txt").is_file()
    {
        Some("pytest".to_string())
    } else {
        None
    };
}

/// Reads an explicit `Verification` command from Project.md.
///
/// This is the project owner's acceptance contract. The command is still
/// subject to Magy's exact-command approval policy before execution.
fn project_verification_command(root: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(root.join("Project.md")).ok()?;
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "Verification" {
            in_section = true;
            continue;
        }
        if in_section
            && matches!(
                trimmed,
                "Goal"
                    | "Requirements"
                    | "Constraints"
                    | "Definition of Done"
                    | "Tasks"
                    | "Current Status"
            )
        {
            break;
        }
        if in_section {
            let command = trimmed.strip_prefix("- ").unwrap_or(trimmed).trim();
            if !command.is_empty() {
                return Some(command.to_string());
            }
        }
    }
    None
}

/// Runs the verification command for the project.
///
/// The agent must be in the Verifying state.
pub fn run_verification(agent: &Agent, command: &str) -> Result<VerificationResult, Error> {
    if agent.state() != &State::Verifying {
        return Err(Error::InvalidStateTransition);
    }

    let root = agent.root().ok_or(Error::Io)?;

    if command.trim().is_empty() {
        return Ok(VerificationResult {
            command: "static project validation".to_string(),
            passed: false,
            stdout:
                "No recognized test runner; generic validation cannot prove acceptance criteria."
                    .to_string(),
            stderr: "Configure an explicit verification command or add a supported project runner."
                .to_string(),
            exit_code: None,
        });
    }

    let output = run_project_command(root, command)?;

    let passed = output.exit_code == Some(0);

    Ok(VerificationResult {
        command: command.to_string(),
        passed,
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: output.exit_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{Agent, Event, Task};
    use tempfile::tempdir;

    #[test]
    fn test_run_verification_pass() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let mut agent = Agent::new();
        agent.transition(Event::Start(root)).unwrap();
        agent
            .transition(Event::TaskSelected(Task {
                id: "1".to_string(),
                description: "T".to_string(),
            }))
            .unwrap();
        agent.transition(Event::ActionDone).unwrap();
        assert_eq!(agent.state(), &State::Verifying);

        let res = run_verification(&agent, "echo pass").unwrap();
        assert!(res.passed);
        assert_eq!(res.exit_code, Some(0));
    }

    #[test]
    fn test_run_verification_fail() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let mut agent = Agent::new();
        agent.transition(Event::Start(root)).unwrap();
        agent
            .transition(Event::TaskSelected(Task {
                id: "1".to_string(),
                description: "T".to_string(),
            }))
            .unwrap();
        agent.transition(Event::ActionDone).unwrap();

        let cmd = if cfg!(windows) { "exit 1" } else { "false" };
        let res = run_verification(&agent, cmd).unwrap();
        assert!(!res.passed);
        assert_ne!(res.exit_code, Some(0));
    }

    #[test]
    fn test_run_verification_invalid_state() {
        let agent = Agent::new();
        let res = run_verification(&agent, "echo fail");
        assert_eq!(res, Err(Error::InvalidStateTransition));
    }

    #[test]
    fn test_recommends_project_specific_runner() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert_eq!(recommended_verification_command(&root), None);
        std::fs::write(root.join("package.json"), "{}").unwrap();
        assert_eq!(
            recommended_verification_command(&root),
            Some("npm test".to_string())
        );
    }

    #[test]
    fn test_project_verification_contract_takes_precedence() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(
            root.join("Project.md"),
            "P\n\nGoal\nG\n\nVerification\n- node scripts/check.js\n\nTasks\n- [ ] T",
        )
        .unwrap();
        std::fs::write(root.join("package.json"), "{}").unwrap();
        assert_eq!(
            recommended_verification_command(&root),
            Some("node scripts/check.js".to_string())
        );
    }

    #[test]
    fn test_static_projects_do_not_run_cargo() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut agent = Agent::new();
        agent.transition(Event::Start(root)).unwrap();
        agent
            .transition(Event::TaskSelected(Task {
                id: "1".to_string(),
                description: "T".to_string(),
            }))
            .unwrap();
        agent.transition(Event::ActionDone).unwrap();

        let result = run_verification(&agent, "").unwrap();
        assert!(!result.passed);
        assert_eq!(result.exit_code, None);
    }
}
