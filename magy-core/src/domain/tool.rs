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

use crate::infrastructure::filesystem::DirEntry;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum ToolRequest {
    ReadFile { path: PathBuf },
    WriteFile { path: PathBuf, content: String },
    ListDirectory { path: PathBuf },
    DiscoverFiles,
    RunCommand { command: String },
    TaskComplete,
}

/// A flat representation of a tool request used for structured output models.
/// All fields are present, using null when they don't apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlatToolRequest {
    pub tool: String,
    pub path: Option<PathBuf>,
    pub content: Option<String>,
    pub command: Option<String>,
}

impl FlatToolRequest {
    pub fn to_tool_request(self) -> Option<ToolRequest> {
        match self.tool.as_str() {
            "read_file" => {
                if self.content.is_none() && self.command.is_none() {
                    self.path.map(|path| ToolRequest::ReadFile { path })
                } else {
                    None
                }
            }

            "write_file" => {
                if self.command.is_none() {
                    if let (Some(path), Some(content)) = (self.path, self.content) {
                        return Some(ToolRequest::WriteFile { path, content });
                    }
                }

                None
            }
            "list_directory" => {
                if self.content.is_none() && self.command.is_none() {
                    self.path.map(|path| ToolRequest::ListDirectory { path })
                } else {
                    None
                }
            }
            "discover_files" => {
                if self.path.is_none() && self.content.is_none() && self.command.is_none() {
                    Some(ToolRequest::DiscoverFiles)
                } else {
                    None
                }
            }
            "run_command" => {
                if self.path.is_none() && self.content.is_none() {
                    self.command
                        .filter(|command| !command.trim().is_empty())
                        .map(|command| ToolRequest::RunCommand { command })
                } else {
                    None
                }
            }
            "task_complete" => {
                if self.path.is_none() && self.content.is_none() && self.command.is_none() {
                    Some(ToolRequest::TaskComplete)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToolResult {
    Text(String),
    Entries(Vec<DirEntry>),
    Paths(Vec<PathBuf>),
    Command(CommandOutput),
    Success,
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_fields_that_do_not_belong_to_write_file() {
        let request = FlatToolRequest {
            tool: "write_file".to_string(),
            path: Some(PathBuf::from("index.html")),
            content: Some("body".to_string()),
            command: Some("task_complete".to_string()),
        };
        assert_eq!(request.to_tool_request(), None);
    }

    #[test]
    fn accepts_minimal_task_complete() {
        let request = FlatToolRequest {
            tool: "task_complete".to_string(),
            path: None,
            content: None,
            command: None,
        };
        assert_eq!(request.to_tool_request(), Some(ToolRequest::TaskComplete));
    }
}
