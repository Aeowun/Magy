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

use std::path::Path;
use std::process::Command;
use crate::Error;
use crate::domain::tool::CommandOutput;

/// Safely runs a command within the project boundary.
///
/// The working directory is set to the project root.
pub fn run_project_command(root: &Path, command_str: &str) -> Result<CommandOutput, Error> {
    let (shell, arg) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };

    let output = Command::new(shell)
        .arg(arg)
        .arg(command_str)
        .current_dir(root)
        .output()
        .map_err(|_| Error::Io)?;

    Ok(CommandOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

    #[test]
    fn test_run_project_command_success() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let output = run_project_command(&root, "echo hello").unwrap();
        assert_eq!(output.stdout.trim(), "hello");
        assert_eq!(output.exit_code, Some(0));
    }

    #[test]
    fn test_run_project_command_cwd() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("test.txt"), "data").unwrap();

        let cmd = if cfg!(windows) { "type test.txt" } else { "cat test.txt" };
        let output = run_project_command(&root, cmd).unwrap();
        assert_eq!(output.stdout.trim(), "data");
    }

    #[test]
    fn test_run_project_command_failure() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let output = run_project_command(&root, "non_existent_command_12345").unwrap();
        assert_ne!(output.exit_code, Some(0));
    }
}
