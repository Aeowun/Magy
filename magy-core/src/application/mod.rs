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

pub mod approval;
pub mod context_assembly;
pub mod coordinator;
pub mod execution;
pub mod planning;
pub mod project_lifecycle;
pub mod reasoning;
pub mod task_lifecycle;
pub mod tool_execution;
pub mod verification_runner;

pub use crate::domain::model::ExecutionTrace;
pub use approval::{resolve_pending_action, ApprovalPolicy, DefaultApprovalPolicy};
pub use context_assembly::assemble_project_context;
pub use coordinator::run_project_workflow;
pub use execution::run_execution_cycle;
pub use planning::plan_execution;
pub use project_lifecycle::{initialize_project, open_project, save_project};
pub use reasoning::{parse_model_action, run_reasoning_step};
pub use task_lifecycle::{complete_current_task, select_task};
pub use tool_execution::execute_tool;
