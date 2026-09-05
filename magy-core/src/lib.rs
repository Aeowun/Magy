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
pub mod boundary;
pub mod infrastructure;
pub mod domain;
pub mod application;

pub use infrastructure::fs::{read_file, write_file, list_directory, discover_files, DirEntry};
pub use domain::project::{ProjectContext, FileContext, FileContent, Project, ProjectTask, TaskStatus, ProjectPlan};
pub use domain::tool::{ToolRequest, ToolResult, CommandOutput};
pub use domain::model::{ModelProvider, ModelRequest, ModelResponse, ModelAction, StepResult};
pub use infrastructure::model::{LmStudioProvider, LmStudioConfig};
pub use application::project::{
    open_project, save_project, select_task, complete_current_task,
    assemble_project_context, plan_execution, execute_tool,
    run_reasoning_step, run_execution_cycle, ExecutionTrace
};

#[derive(Debug, PartialEq, Clone)]
pub enum Error {
    /// Attempted to access a path outside the project boundary.
    OutsideBoundary,
    /// A generic filesystem error occurred.
    Io,
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
    /// An error occurred while communicating with the model provider.
    ModelError(String),
}
