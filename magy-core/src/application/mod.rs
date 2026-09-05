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

pub mod project_lifecycle;
pub mod task_lifecycle;
pub mod context_assembly;
pub mod planning;
pub mod reasoning;
pub mod approval;
pub mod tool_execution;
pub mod execution;
pub mod verification_runner;
pub mod coordinator;

pub use project_lifecycle::{open_project, save_project, initialize_project};
pub use task_lifecycle::{select_task, complete_current_task};
pub use context_assembly::assemble_project_context;
pub use planning::plan_execution;
pub use reasoning::{run_reasoning_step, parse_model_action};
pub use approval::{ApprovalPolicy, DefaultApprovalPolicy, resolve_pending_action};
pub use tool_execution::execute_tool;
pub use execution::run_execution_cycle;
pub use coordinator::run_project_workflow;
pub use crate::domain::model::ExecutionTrace;
