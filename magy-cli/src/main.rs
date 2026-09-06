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

use magy_core::{
    open_project, recommended_verification_command, resolve_pending_action, run_project_workflow,
    select_task, ApprovalStatus, DefaultApprovalPolicy, ExecutionTrace, LmStudioConfig,
    LmStudioProvider, RunResult, ToolRequest,
};
use std::io::{self, Write};
use std::path::PathBuf;

const SYSTEM_PROMPT: &str = r#"You are Magy, a privacy-first software engineering agent operating inside an existing user-selected project.

Return exactly one JSON object matching the tool schema. Use only the listed tools.
Never invent tools such as git, bash, shell, powershell, or terminal.
Do not initialize Git, create a repository, install packages, access the network,
change permissions, delete files, or perform unrelated setup unless the active task explicitly requires it.
Use run_command only for an allowlisted command that directly advances the active task.
If a command is denied, choose a different action instead of repeating it.
All fields (tool, path, content, command) are required. Use null when not applicable.
For write_file, provide path and content only; command must be null. For run_command,
provide command only; path and content must be null. For task_complete and
discover_files, all three optional fields must be null.
Use git_status and git_diff to inspect the real repository state before claiming progress.
Only use task_complete after the requested work has actually been performed and
no required action was denied or failed. A model assertion is not verification.
All tool paths are relative to the selected project root; use "index.html", not
"/index.html". If generic verification is unavailable, report that limitation
instead of claiming success."#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = CliConfig::from_args(std::env::args().collect())?;

    let provider = LmStudioProvider::new(config.lm_config.clone());
    let verification_command = if config.verification_command == "auto" {
        recommended_verification_command(&config.project_path).unwrap_or_default()
    } else {
        config.verification_command.clone()
    };
    let policy = DefaultApprovalPolicy::default().allow_command(verification_command.clone());
    let mut trace = ExecutionTrace::new();
    let mut last_step_count = 0;

    println!(
        "Magy\nProject: {}\nVerification: {}\n",
        config.project_path.display(),
        if verification_command.is_empty() {
            "static project validation"
        } else {
            &verification_command
        }
    );

    // 1. Try to open project, initialize if missing
    match open_project(config.project_path.clone()) {
        Ok(_) => {}
        Err(magy_core::Error::FileNotFound) => {
            println!("Project.md not found. Initializing new project...");
            let goal = "Build a small Rust command-line calculator."; // For testing
            let p = magy_core::initialize_project(config.project_path.clone(), &provider, goal)?;
            println!("Project initialized: {}\n", p.name);
        }
        Err(e) => return Err(e.into()),
    };

    loop {
        let result = run_project_workflow(
            config.project_path.clone(),
            &provider,
            &policy,
            &verification_command,
            &config.system_prompt,
            config.max_steps,
            config.max_verifications,
            &mut trace,
        )?;

        display_new_steps(&trace, &mut last_step_count);

        if result.project_completed {
            println!(
                "\nProject complete. Tasks completed: {}",
                result.completed_task_ids.len()
            );
            break;
        }

        if result.stop_reason == "Action requires approval" {
            handle_approval(&config, &result, &mut trace)?;
            continue;
        }

        println!("\nExecution stopped: {}", result.stop_reason);
        break;
    }

    Ok(())
}

struct CliConfig {
    project_path: PathBuf,
    verification_command: String,
    lm_config: LmStudioConfig,
    system_prompt: String,
    max_steps: usize,
    max_verifications: usize,
}

impl CliConfig {
    fn from_args(args: Vec<String>) -> Result<Self, String> {
        if args.len() < 2 {
            return Err(
                "Usage: magy <project-path> [verification-command|auto] [max-steps] [max-verifications]"
                    .to_string(),
            );
        }
        let project_path = PathBuf::from(&args[1]);
        let verification_command = args.get(2).cloned().unwrap_or_else(|| "auto".to_string());
        let max_steps = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
        let max_verifications = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(3);

        Ok(Self {
            project_path,
            verification_command,
            lm_config: LmStudioConfig {
                base_url: "http://localhost:1234/v1".to_string(),
                model_name: "nvidia/nemotron-3-nano-4b".to_string(),
            },
            system_prompt: SYSTEM_PROMPT.to_string(),
            max_steps,
            max_verifications,
        })
    }
}

fn display_new_steps(trace: &ExecutionTrace, last_count: &mut usize) {
    for i in *last_count..trace.steps.len() {
        let step = &trace.steps[i];
        println!("\n--- Step {} ---", i + 1);
        println!("Model: {}", step.model_response.content);
        if let Some(record) = &step.action_record {
            println!("Action: {:?}", record.request);
            if record.request != ToolRequest::TaskComplete {
                println!("Outcome: {:?}", record.outcome);
            }
        }
        if let Some(ver) = &step.verification {
            println!(
                "Verification: {}",
                if ver.passed { "PASSED" } else { "FAILED" }
            );
            if !ver.passed {
                println!(
                    "Exit Code: {:?}\nOutput:\n{}{}",
                    ver.exit_code, ver.stdout, ver.stderr
                );
            }
        }
    }
    *last_count = trace.steps.len();
}

fn handle_approval(
    config: &CliConfig,
    result: &RunResult,
    trace: &mut ExecutionTrace,
) -> io::Result<()> {
    let (idx, record) = trace
        .steps
        .iter()
        .enumerate()
        .find_map(|(i, s)| {
            s.action_record
                .as_ref()
                .filter(|r| r.approval_status == ApprovalStatus::Pending)
                .map(|r| (i, r))
        })
        .expect("Pending action missing");

    println!(
        "\nAction requires approval\nTool: {:?}\nApprove? [y/N]: ",
        record.request
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let approved = input.trim().to_lowercase() == "y";

    if let Some(task_id) = &result.active_task_id {
        let (mut agent, project) = open_project(config.project_path.clone())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
        select_task(&mut agent, &project, task_id)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
        resolve_pending_action(&agent, trace, idx, approved)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{:?}", e)))?;
        println!(
            "Action {}.",
            if approved {
                "approved and executed"
            } else {
                "denied"
            }
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_parsing() {
        let args = vec![
            "magy".to_string(),
            "./path".to_string(),
            "test-cmd".to_string(),
        ];
        let config = CliConfig::from_args(args).unwrap();
        assert_eq!(config.project_path, PathBuf::from("./path"));
        assert_eq!(config.verification_command, "test-cmd");
    }

    #[test]
    fn test_config_missing_args() {
        let args = vec!["magy".to_string()];
        assert!(CliConfig::from_args(args).is_err());
    }
}
