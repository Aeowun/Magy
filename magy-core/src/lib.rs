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

// Magy Core - Domain Logic
pub mod application;
pub mod boundary;
pub mod domain;
pub mod infrastructure;

pub use application::verification_runner::recommended_verification_command;
pub use application::{
    assemble_project_context, complete_current_task, execute_tool, initialize_project,
    open_project, plan_execution, resolve_pending_action, run_execution_cycle,
    run_project_workflow, run_reasoning_step, save_project, select_task, ApprovalPolicy,
    DefaultApprovalPolicy,
};
pub use domain::model::{
    ActionRecord, ApprovalStatus, ExecutionOutcome, ExecutionTrace, ModelAction, ModelProvider,
    ModelRequest, ModelResponse, RunResult, StepResult,
};
pub use domain::project::{
    FileContent, FileContext, Project, ProjectContext, ProjectPlan, ProjectTask, TaskStatus,
};
pub use domain::tool::{CommandOutput, ToolRequest, ToolResult};
pub use infrastructure::filesystem::{
    discover_files, list_directory, read_file, write_file, DirEntry,
};
pub use infrastructure::model::{LmStudioConfig, LmStudioProvider};

use std::fmt;

#[derive(Debug, PartialEq, Clone)]
pub enum Error {
    /// Attempted to access a path outside the project boundary.
    OutsideBoundary,
    /// A generic filesystem error occurred.
    Io,
    /// The requested file was not found.
    FileNotFound,
    /// An invalid state transition was requested for the agent.
    InvalidStateTransition,
    /// The project goal section is missing.
    MissingGoal,
    /// The project tasks section is missing.
    MissingTasks,
    /// The requested task was not found in the project.
    TaskNotFound,
    /// The requested task is already marked as done.
    TaskAlreadyDone,
    /// No task is currently active for the agent.
    NoActiveTask,
    /// The requested action was not found in the trace.
    ActionNotFound,
    /// The requested action is not in a pending state.
    ActionNotPending,
    /// An error occurred while communicating with the model provider.
    ModelError(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for Error {}
