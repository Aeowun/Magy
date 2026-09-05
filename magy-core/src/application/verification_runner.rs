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

/// Runs the verification command for the project.
///
/// The agent must be in the Verifying state.
pub fn run_verification(agent: &Agent, command: &str) -> Result<VerificationResult, Error> {
    if agent.state() != &State::Verifying {
        return Err(Error::InvalidStateTransition);
    }

    let root = agent.root().ok_or(Error::Io)?;

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
}
