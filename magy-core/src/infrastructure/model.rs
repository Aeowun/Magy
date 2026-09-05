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

use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::Error;
use crate::domain::model::{ModelProvider, ModelRequest, ModelResponse};
use crate::domain::project::FileContent;

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
    fn ask(&self, request: ModelRequest) -> Result<ModelResponse, Error> {
        let system_message = OpenAiMessage {
            role: "system".to_string(),
            content: request.system_prompt,
        };

        let mut user_content = format!("Project Name: {}\nGoal: {}\n\n",
            request.context.project.name,
            request.context.project.goal
        );

        if let Some(task) = &request.task {
            user_content.push_str(&format!("Active Task: [{}] {}\n\n", task.id, task.description));
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
                        "enum": ["read_file", "write_file", "list_directory", "discover_files", "run_command", "task_complete"]
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

        let url = format!("{}/chat/completions", self.config.base_url.trim_end_matches('/'));

        let response = self.client.post(url)
            .json(&openai_req)
            .send()
            .map_err(|e| Error::ModelError(format!("Network error: {}", e)))?;

        if !response.status().is_success() {
            return Err(Error::ModelError(format!("Provider returned status {}", response.status())));
        }

        let body: OpenAiResponse = response.json().map_err(|e| Error::ModelError(format!("JSON parse error: {}", e)))?;

        let content = body.choices.first()
            .map(|c| c.message.content.clone())
            .ok_or(Error::ModelError("No choices in response".to_string()))?;

        Ok(ModelResponse { content })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use mockito::Server;
    use crate::domain::project::{Project, FileContext, ProjectContext};

    #[test]
    fn test_lm_studio_request_parsing() {
        let mut server = Server::new();
        let url = server.url();

        let mock = server.mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Json(json!({
                "model": "test-model",
                "messages": [
                    { "role": "system", "content": "You are a helper." },
                    { "role": "user", "content": "Project Name: Test\nGoal: Test goal\n\nFiles:\n--- src/lib.rs ---\nfn main() {}\n\n" }
                ],
                "temperature": 0.0,
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "request",
                        "strict": true,
                        "schema": {
                            "type": "object",
                            "properties": {
                                "tool": {
                                    "type": "string",
                                    "enum": ["read_file", "write_file", "list_directory", "discover_files", "run_command", "task_complete"]
                                },
                                "path": { "type": ["string", "null"] },
                                "content": { "type": ["string", "null"] },
                                "command": { "type": ["string", "null"] }
                            },
                            "required": ["tool", "path", "content", "command"],
                            "additionalProperties": false
                        }
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "{\"tool\": \"discover_files\", \"path\": null, \"content\": null, \"command\": null}"
                    }
                }]
            }"#)
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
                },
                files: vec![
                    FileContext {
                        path: PathBuf::from("src/lib.rs"),
                        content: FileContent::Text("fn main() {}".to_string()),
                    }
                ],
            },
            task: None,
            plan: None,
            history: vec![],
            schema: None,
        };

        let response = provider.ask(request).unwrap();
        assert_eq!(response.content, "{\"tool\": \"discover_files\", \"path\": null, \"content\": null, \"command\": null}");
        mock.assert();
    }

    #[test]
    fn test_lm_studio_custom_schema() {
        let mut server = Server::new();
        let url = server.url();

        let custom_schema = json!({
            "type": "object",
            "properties": { "val": { "type": "number" } },
            "required": ["val"]
        });

        let mock = server.mock("POST", "/chat/completions")
            .match_body(mockito::Matcher::Json(json!({
                "model": "test-model",
                "messages": [
                    { "role": "system", "content": "S" },
                    { "role": "user", "content": "Project Name: P\nGoal: G\n\nFiles:\n" }
                ],
                "temperature": 0.0,
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "request",
                        "strict": true,
                        "schema": custom_schema
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"choices": [{"message": {"role": "assistant", "content": "{\"val\": 42}"}}]}"#)
            .create();

        let config = LmStudioConfig { base_url: url, model_name: "test-model".to_string() };
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
                },
                files: vec![],
            },
            task: None,
            plan: None,
            history: vec![],
            schema: Some(custom_schema),
        };

        let response = provider.ask(request).unwrap();
        assert_eq!(response.content, "{\"val\": 42}");
        mock.assert();
    }

    #[test]
    fn test_lm_studio_error_handling() {
        let mut server = Server::new();
        let url = server.url();

        let _mock = server.mock("POST", "/chat/completions")
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
