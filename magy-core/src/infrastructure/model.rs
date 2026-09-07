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

use crate::domain::model::{
    ModelProvider, ModelRequest, ModelResponse, PlannerRequest, PlannerResponse,
};
use crate::domain::project::FileContent;
use crate::Error;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct LmStudioConfig {
    pub base_url: String,
    pub model_name: String,
}

pub struct LmStudioProvider {
    config: LmStudioConfig,
    client: reqwest::blocking::Client,
}

impl LmStudioProvider {
    pub fn new(config: LmStudioConfig) -> Self {
        Self {
            config,
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new()),
        }
    }

    pub fn ask_chat(
        &self,
        message: &str,
        history: &[(String, String)],
        project: Option<&crate::domain::project::Project>,
    ) -> Result<String, Error> {
        let mut system_content = "You are Magy, a concise and helpful local software engineering assistant. \
Answer conversational questions directly. Do not emit tool calls or claim to have changed files.".to_string();

        if let Some(p) = project {
            system_content.push_str(&format!(
                "\n\nContext:\nYou are helping the user with a project named '{}'.\nGoal: {}\n",
                p.name, p.goal
            ));
            if !p.tasks.is_empty() {
                system_content.push_str("Current Tasks:\n");
                for t in &p.tasks {
                    let status = match t.status {
                        crate::domain::project::TaskStatus::Open => "[ ]",
                        crate::domain::project::TaskStatus::Done => "[x]",
                    };
                    system_content.push_str(&format!("{} {}\n", status, t.description));
                }
            }
        }

        let mut messages = vec![OpenAiMessage {
            role: "system".to_string(),
            content: system_content,
        }];
        for (role, content) in history.iter().take(12) {
            messages.push(OpenAiMessage {
                role: role.clone(),
                content: content.clone(),
            });
        }
        messages.push(OpenAiMessage {
            role: "user".to_string(),
            content: message.to_string(),
        });

        let request = OpenAiRequest {
            model: self.config.model_name.clone(),
            messages,
            temperature: 0.2,
            response_format: None,
        };
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let response = self
            .client
            .post(url)
            .json(&request)
            .send()
            .map_err(|e| Error::ModelError(format!("Network error: {}", e)))?;
        if !response.status().is_success() {
            return Err(Error::ModelError(format!(
                "Provider returned status {}",
                response.status()
            )));
        }
        let body: OpenAiResponse = response
            .json()
            .map_err(|e| Error::ModelError(format!("JSON parse error: {}", e)))?;
        body.choices
            .first()
            .map(|choice| choice.message.content.clone())
            .ok_or(Error::ModelError("No choices in response".to_string()))
    }
}

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<OpenAiResponseFormat>,
}

#[derive(Serialize)]
struct OpenAiResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    json_schema: Option<OpenAiJsonSchema>,
}

#[derive(Serialize)]
struct OpenAiJsonSchema {
    name: String,
    strict: bool,
    schema: serde_json::Value,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct OpenAiResponseMessage {
    content: String,
}

impl ModelProvider for LmStudioProvider {
    fn plan(&self, request: PlannerRequest) -> Result<PlannerResponse, Error> {
        let system_message = OpenAiMessage {
            role: "system".to_string(),
            content: "You are the Magy Project Architect (Qwen-based). \
Your role is to decompose the user's goal into a structured, dependency-aware plan. \
You do not execute tools. You only produce a valid, high-level Project Plan. \
Return exactly one JSON object matching the PlannerResponse schema. \
Ensure unique task IDs, clear acceptance criteria for every task, and logical sequencing. \
Do not claim any task is already complete.".to_string(),
        };

        let mut user_content = format!(
            "Project Goal: {}\n\n",
            request.goal
        );
        if !request.requirements.is_empty() {
            user_content.push_str("Requirements:\n");
            for r in &request.requirements {
                user_content.push_str(&format!("- {}\n", r));
            }
        }
        // ... (Include other PlannerRequest fields if needed)

        let user_message = OpenAiMessage {
            role: "user".to_string(),
            content: user_content,
        };

        let schema = json!({
            "type": "object",
            "properties": {
                "plan_version": { "type": "integer" },
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "description": { "type": "string" },
                            "status": { "type": "string", "enum": ["open"] },
                            "acceptance_criteria": {
                                "type": "array",
                                "items": { "type": "string" }
                            }
                        },
                        "required": ["id", "description", "status", "acceptance_criteria"]
                    }
                }
            },
            "required": ["plan_version", "tasks"]
        });

        let openai_req = OpenAiRequest {
            model: self.config.model_name.clone(),
            messages: vec![system_message, user_message],
            temperature: 0.1,
            response_format: Some(OpenAiResponseFormat {
                format_type: "json_schema".to_string(),
                json_schema: Some(OpenAiJsonSchema {
                    name: "planner_response".to_string(),
                    strict: true,
                    schema,
                }),
            }),
        };

        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );

        let response = self
            .client
            .post(url)
            .json(&openai_req)
            .send()
            .map_err(|e| Error::ModelError(format!("Network error: {}", e)))?;

        let body: OpenAiResponse = response
            .json()
            .map_err(|e| Error::ModelError(format!("JSON parse error: {}", e)))?;

        let content = body
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or(Error::ModelError("No choices in response".to_string()))?;

        // Parse to PlannerResponse
        let resp: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| Error::ParseError(format!("Invalid planner JSON: {}", e)))?;

        let tasks_val = resp.get("tasks").ok_or(Error::ParseError("Missing tasks".to_string()))?;
        let mut tasks = Vec::new();

        if let Some(arr) = tasks_val.as_array() {
            for t in arr {
                tasks.push(crate::domain::project::ProjectTask {
                    id: t.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    description: t.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    status: crate::domain::project::TaskStatus::Open,
                    acceptance_criteria: t.get("acceptance_criteria").and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|s| s.as_str().map(|ss| ss.to_string())).collect())
                        .unwrap_or_default(),
                    evidence: vec![],
                });
            }
        }

        Ok(PlannerResponse {
            plan_version: resp.get("plan_version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
            tasks,
        })
    }

    fn ask(&self, request: ModelRequest) -> Result<ModelResponse, Error> {
        let system_message = OpenAiMessage {
            role: "system".to_string(),
            content: "You are the Magy Bounded Executor (Nemotron-based). \
You receive exactly one active task and its acceptance criteria. \
Your goal is to perform the necessary actions (read, write, run commands) to satisfy the task's criteria. \
All tool paths must be relative to the project root (e.g., 'src/main.rs', NOT '/project/src/main.rs'). \
Return exactly one JSON object matching the ToolRequest schema. \
All four fields (tool, path, content, command) are REQUIRED in the output. \
Use null for fields that do not apply to the chosen tool. \
For write_file: provide path and content; command must be null. \
For run_command: provide command; path and content must be null. \
For task_complete: all three optional fields must be null. \
You cannot create new tasks or modify the project plan. \
You cannot declare project completion.".to_string(),
        };

        let mut user_content = format!(
            "Project Name: {}\nGoal: {}\n\n",
            request.context.project.name, request.context.project.goal
        );

        if let Some(task) = &request.task {
            user_content.push_str(&format!(
                "Active Task: [{}] {}\n\n",
                task.id, task.description
            ));
            user_content.push_str(
                "Task contract:\n\
- Work only on this active task.\n\
- Inspect existing files before rewriting them.\n\
- Make one concrete change, then use the verification evidence before making another change.\n\
- Do not repeat an identical action unless new evidence requires it.\n\
- Request task_complete only after the task's required artifact and verification are complete.\n\n",
            );
        }

        if !request.context.project.requirements.is_empty() {
            user_content.push_str("Project requirements:\n");
            for requirement in &request.context.project.requirements {
                user_content.push_str(&format!("- {}\n", requirement));
            }
            user_content.push('\n');
        }
        if !request.context.project.definition_of_done.is_empty() {
            user_content.push_str("Definition of done:\n");
            for criterion in &request.context.project.definition_of_done {
                user_content.push_str(&format!("- {}\n", criterion));
            }
            user_content.push('\n');
        }

        if let Some(plan) = &request.plan {
            user_content.push_str("Current Plan:\n");
            for t in &plan.tasks {
                user_content.push_str(&format!("- {}\n", t.description));
            }
            user_content.push('\n');
        }

        user_content.push_str("Files:\n");
        for file in &request.context.files {
            let path_str = file.path.to_string_lossy();
            match &file.content {
                FileContent::Text(t) => {
                    user_content.push_str(&format!("--- {} ---\n{}\n\n", path_str, t));
                }
                FileContent::Directory => {
                    user_content.push_str(&format!("(Directory) {}\n", path_str));
                }
                FileContent::Unreadable(reason) => {
                    user_content.push_str(&format!("(Unreadable) {}: {}\n", path_str, reason));
                }
            }
        }

        if !request.history.is_empty() {
            user_content.push_str("\nExecution History:\n");
            for step in &request.history {
                user_content.push_str(&format!("Model: {}\n", step.model_response.content));
                if let Some(record) = &step.action_record {
                    user_content.push_str(&format!("Action: {:?}\n", record.request));
                    user_content.push_str(&format!("Outcome: {:?}\n", record.outcome));
                }
                if let Some(ver) = &step.verification {
                    user_content.push_str(&format!(
                        "\nVerification Result: {}\nCommand: {}\nExit Code: {:?}\nOutput:\n{}\n{}\n",
                        if ver.passed { "PASSED" } else { "FAILED" },
                        ver.command,
                        ver.exit_code,
                        ver.stdout,
                        ver.stderr
                    ));
                }
            }
            user_content.push('\n');
        }

        let user_message = OpenAiMessage {
            role: "user".to_string(),
            content: user_content,
        };

        let schema = request.schema.clone().unwrap_or_else(|| {
            json!({
                "type": "object",
                "properties": {
                    "tool": {
                        "type": "string",
                        "enum": ["read_file", "write_file", "list_directory", "discover_files", "git_status", "git_diff", "run_command", "task_complete"]
                    },
                    "path": { "type": ["string", "null"] },
                    "content": { "type": ["string", "null"] },
                    "command": { "type": ["string", "null"] }
                },
                "required": ["tool", "path", "content", "command"],
                "additionalProperties": false
            })
        });

        let openai_req = OpenAiRequest {
            model: self.config.model_name.clone(),
            messages: vec![system_message, user_message],
            temperature: 0.0,
            response_format: Some(OpenAiResponseFormat {
                format_type: "json_schema".to_string(),
                json_schema: Some(OpenAiJsonSchema {
                    name: "request".to_string(),
                    strict: true,
                    schema,
                }),
            }),
        };

        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );

        let response = self
            .client
            .post(url)
            .json(&openai_req)
            .send()
            .map_err(|e| Error::ModelError(format!("Network error: {}", e)))?;

        if !response.status().is_success() {
            return Err(Error::ModelError(format!(
                "Provider returned status {}",
                response.status()
            )));
        }

        let body: OpenAiResponse = response
            .json()
            .map_err(|e| Error::ModelError(format!("JSON parse error: {}", e)))?;

        let content = body
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or(Error::ModelError("No choices in response".to_string()))?;

        Ok(ModelResponse { content })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::{FileContext, Project, ProjectContext};
    use mockito::Server;
    use std::path::PathBuf;

    #[test]
    fn test_lm_studio_request_parsing() {
        let mut server = Server::new();
        let url = server.url();

        let _mock = server.mock("POST", "/chat/completions")
            .with_status(501) // Not Implemented for testing
            .create();

        let config = LmStudioConfig {
            base_url: url,
            model_name: "test-model".to_string(),
        };
        let provider = LmStudioProvider::new(config);

        let request = ModelRequest {
            system_prompt: "You are a helper.".to_string(),
            context: ProjectContext {
                project: Project {
                    name: "Test".to_string(),
                    goal: "Test goal".to_string(),
                    requirements: vec![],
                    constraints: vec![],
                    definition_of_done: vec![],
                    tasks: vec![],
                    current_status: "".to_string(),
                    plan_version: 0,
                    plan_created_at_ms: None,
                    replan_count: 0,
                    replan_reason: None,
                },
                files: vec![FileContext {
                    path: PathBuf::from("src/lib.rs"),
                    content: FileContent::Text("fn main() {}".to_string()),
                }],
            },
            task: None,
            plan: None,
            history: vec![],
            schema: None,
        };

        let result = provider.ask(request);
        assert!(result.is_err());
    }

    #[test]
    fn test_lm_studio_custom_schema() {
        let mut server = Server::new();
        let url = server.url();

        let _mock = server.mock("POST", "/chat/completions")
            .with_status(501)
            .create();

        let config = LmStudioConfig {
            base_url: url,
            model_name: "test-model".to_string(),
        };
        let provider = LmStudioProvider::new(config);

        let request = ModelRequest {
            system_prompt: "S".to_string(),
            context: ProjectContext {
                project: Project {
                    name: "P".to_string(),
                    goal: "G".to_string(),
                    requirements: vec![],
                    constraints: vec![],
                    definition_of_done: vec![],
                    tasks: vec![],
                    current_status: "".to_string(),
                    plan_version: 0,
                    plan_created_at_ms: None,
                    replan_count: 0,
                    replan_reason: None,
                },
                files: vec![],
            },
            task: None,
            plan: None,
            history: vec![],
            schema: Some(json!({})),
        };

        let result = provider.ask(request);
        assert!(result.is_err());
    }

    #[test]
    fn test_lm_studio_error_handling() {
        let mut server = Server::new();
        let url = server.url();

        let _mock = server
            .mock("POST", "/chat/completions")
            .with_status(500)
            .create();

        let config = LmStudioConfig {
            base_url: url,
            model_name: "nvidia/nemotron-3-nano-4b".to_string(),
        };
        let provider = LmStudioProvider::new(config);

        let request = ModelRequest {
            system_prompt: "S".to_string(),
            context: ProjectContext {
                project: Project {
                    name: "P".to_string(),
                    goal: "G".to_string(),
                    requirements: vec![],
                    constraints: vec![],
                    definition_of_done: vec![],
                    tasks: vec![],
                    current_status: "".to_string(),
                    plan_version: 0,
                    plan_created_at_ms: None,
                    replan_count: 0,
                    replan_reason: None,
                },
                files: vec![],
            },
            task: None,
            plan: None,
            history: vec![],
            schema: None,
        };

        let result = provider.ask(request);
        match result {
            Err(Error::ModelError(m)) => assert!(m.contains("Provider returned status 500")),
            _ => panic!("Expected ModelError"),
        }
    }
}
