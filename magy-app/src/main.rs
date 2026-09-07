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

use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use futures_util::FutureExt;
use magy_core::{
    load_run_snapshot, open_project, recommended_verification_command, recover_orphaned_snapshot,
    resolve_pending_action, run_project_workflow, save_run_snapshot, Agent, CancellationReason,
    DefaultApprovalPolicy, Error as MagyError, ExecutionTrace, FailureReason, LmStudioConfig,
    LmStudioProvider, Project, RunOutcome, RunResult, RunSnapshot, RunState,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::services::ServeDir;

const WORKER_OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(95);

fn planner_config() -> LmStudioConfig {
    LmStudioConfig {
        base_url: std::env::var("MAGY_PLANNER_BASE_URL")
            .or_else(|_| std::env::var("MAGY_BASE_URL"))
            .unwrap_or_else(|_| "http://localhost:1234/v1".to_string()),
        model_name: std::env::var("MAGY_PLANNER_MODEL")
            .or_else(|_| std::env::var("MAGY_MODEL"))
            .unwrap_or_else(|_| "nvidia/nemotron-3-nano-4b".to_string()),
    }
}

fn executor_config() -> LmStudioConfig {
    LmStudioConfig {
        base_url: std::env::var("MAGY_EXECUTOR_BASE_URL")
            .or_else(|_| std::env::var("MAGY_BASE_URL"))
            .unwrap_or_else(|_| "http://localhost:1234/v1".to_string()),
        model_name: std::env::var("MAGY_EXECUTOR_MODEL")
            .or_else(|_| std::env::var("MAGY_MODEL"))
            .unwrap_or_else(|_| "nvidia/nemotron-3-nano-4b".to_string()),
    }
}

const MAX_EVENT_HISTORY: usize = 256;

fn publish_event_locked(state: &mut AppState, event: UiEvent) {
    state.event_seq = state.event_seq.saturating_add(1);
    let envelope = UiEventEnvelope {
        seq: state.event_seq,
        event,
    };
    state.event_history.push_back(envelope.clone());
    while state.event_history.len() > MAX_EVENT_HISTORY {
        state.event_history.pop_front();
    }
}

async fn publish_event(state: &SharedState, event: UiEvent) {
    let mut s = state.lock().await;
    publish_event_locked(&mut s, event);
}

fn publish_terminal_locked(state: &mut AppState) {
    if state.terminal_event_emitted {
        return;
    }
    if let Some(outcome) = state.trace.run.outcome() {
        publish_event_locked(state, UiEvent::RunFinished(outcome.clone()));
        state.terminal_event_emitted = true;
    }
}

fn persist_snapshot_locked(state: &AppState) {
    if let Some(root) = &state.root {
        if let Some(project) = &state.project {
            let snapshot = RunSnapshot::new(
                state.event_seq,
                state.worker_active,
                state.trace.clone(),
                project.clone(),
                state.agent.clone(),
            );
            let _ = save_run_snapshot(root, &snapshot);
        }
    }
}

struct AppState {
    root: Option<PathBuf>,
    project: Option<Project>,
    agent: Option<Agent>,
    trace: ExecutionTrace,
    worker_active: bool,
    cancel_requested: Arc<AtomicBool>,
    event_seq: u64,
    event_history: VecDeque<UiEventEnvelope>,
    terminal_event_emitted: bool,
    auto_approve_tools: bool,
    chat_history: Vec<(String, String)>,
}

type SharedState = Arc<Mutex<AppState>>;

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum UiEvent {
    ProjectLoaded(UiProject),
    AgentStateChanged(RunState),
    Step {
        step: serde_json::Value,
        index: usize,
    },
    RunFinished(RunOutcome),
    Error(String),
}

#[derive(Serialize, Clone, Debug)]
struct UiEventEnvelope {
    seq: u64,
    event: UiEvent,
}

#[derive(Serialize, Clone, Debug)]
struct UiProject {
    name: String,
    goal: String,
    tasks: Vec<UiTask>,
}

#[derive(Serialize, Clone, Debug)]
struct UiTask {
    id: String,
    description: String,
    status: String,
}

fn to_ui_project(project: &Project) -> UiProject {
    UiProject {
        name: project.name.clone(),
        goal: project.goal.clone(),
        tasks: project
            .tasks
            .iter()
            .map(|t| UiTask {
                id: t.id.clone(),
                description: t.description.clone(),
                status: format!("{:?}", t.status),
            })
            .collect(),
    }
}

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

#[tokio::main]
async fn main() {
    let state = Arc::new(Mutex::new(AppState {
        root: None,
        project: None,
        agent: None,
        trace: ExecutionTrace::new(),
        worker_active: false,
        cancel_requested: Arc::new(AtomicBool::new(false)),
        event_seq: 0,
        event_history: VecDeque::new(),
        terminal_event_emitted: false,
        auto_approve_tools: false,
        chat_history: Vec::new(),
    }));

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/api/load-project", post(load_project))
        .route("/api/initialize", post(initialize_project))
        .route("/api/run", post(run_agent))
        .route("/api/cancel", post(cancel_run))
        .route("/api/events", get(get_events))
        .route("/api/resolve", post(resolve_action))
        .route("/api/github-info", get(github_info))
        .route("/api/chat", post(chat))
        .route("/api/settings", post(update_settings))
        // Serve everything in magy-ui at the root so index.html can find app.js/style.css
        .fallback_service(ServeDir::new("magy-ui"))
        .with_state(state);

    println!("Starting Magy at http://127.0.0.1:3000");
    let addr = "0.0.0.0:3000".parse().unwrap();

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn root_handler() -> impl IntoResponse {
    Html(include_str!("../../magy-ui/index.html"))
}

async fn load_project(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let folder = tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Open Magy Project")
            .pick_folder()
    })
    .await
    .unwrap();

    let Some(path) = folder else {
        return Json(serde_json::json!({ "status": "cancelled" }));
    };

    let mut s = state.lock().await;
    match open_project(path.clone()) {
        Ok((agent, project)) => {
            s.root = Some(path.clone());
            s.project = Some(project.clone());
            s.agent = Some(agent);
            let ui_project = to_ui_project(&project);

            if let Ok(Some(mut snapshot)) = load_run_snapshot(&path) {
                if recover_orphaned_snapshot(&mut snapshot, 300_000) {
                    let _ = save_run_snapshot(&path, &snapshot);
                }
                s.trace = snapshot.trace;
                s.worker_active = snapshot.worker_active;
                s.event_seq = snapshot.sequence;
                if let Some(a) = snapshot.agent {
                    s.agent = Some(a);
                }
            } else {
                s.trace = ExecutionTrace::new();
                s.worker_active = false;
            }

            publish_event_locked(&mut s, UiEvent::ProjectLoaded(ui_project.clone()));
            Json(serde_json::json!({ "status": "success", "project": ui_project }))
        }
        Err(MagyError::FileNotFound) => {
            s.root = Some(path);
            s.project = None;
            s.agent = None;
            s.trace = ExecutionTrace::new();
            s.worker_active = false;
            Json(serde_json::json!({ "status": "error", "message": "Project.md not found" }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("{:?}", e) })),
    }
}

async fn cancel_run(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let s = state.lock().await;
    if s.worker_active {
        s.cancel_requested.store(true, Ordering::Release);
        return Json(serde_json::json!({ "status": "cancellation_requested" }));
    }
    Json(serde_json::json!({ "status": "not_running" }))
}

#[derive(Deserialize)]
struct InitRequest {
    goal: String,
}

async fn initialize_project(
    State(state): State<SharedState>,
    Json(payload): Json<InitRequest>,
) -> Json<serde_json::Value> {
    let Some(root) = state.lock().await.root.clone() else {
        return Json(serde_json::json!({ "status": "error", "message": "No directory selected" }));
    };

    let config = planner_config();
    let goal = payload.goal;

    let res = tokio::time::timeout(
        WORKER_OPERATION_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            let provider = LmStudioProvider::new(config);
            magy_core::initialize_project(root, &provider, &goal)
        }),
    )
    .await;

    match res {
        Ok(Ok(Ok(project))) => {
            let mut s = state.lock().await;
            s.project = Some(project.clone());
            let ui_project = to_ui_project(&project);
            s.trace = ExecutionTrace::new();
            s.worker_active = false;
            s.terminal_event_emitted = false;
            s.event_history.clear();
            publish_event_locked(&mut s, UiEvent::ProjectLoaded(ui_project.clone()));
            publish_event_locked(&mut s, UiEvent::AgentStateChanged(RunState::Idle));
            persist_snapshot_locked(&s);
            Json(serde_json::json!({ "status": "success", "project": ui_project }))
        }
        Ok(Ok(Err(e))) => Json(serde_json::json!({ "status": "error", "message": format!("{:?}", e) })),
        Ok(Err(e)) => Json(serde_json::json!({ "status": "error", "message": format!("Initialization worker failed: {}", e) })),
        Err(_) => Json(serde_json::json!({ "status": "error", "message": "Initialization timed out" })),
    }
}

async fn mark_run_failed(state: &SharedState, message: impl Into<String>) {
    let mut s = state.lock().await;
    s.worker_active = false;
    if s.trace.run.state().is_terminal() {
        publish_terminal_locked(&mut s);
        persist_snapshot_locked(&s);
        return;
    }
    s.trace.run.fail(FailureReason::Internal(message.into()));
    publish_terminal_locked(&mut s);
    persist_snapshot_locked(&s);
}

async fn run_agent(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let mut s = state.lock().await;
    if s.worker_active {
        return Json(serde_json::json!({ "status": "error", "message": "Agent is already running" }));
    }
    let root = match s.root.clone() {
        Some(root) => root,
        None => return Json(serde_json::json!({ "status": "error", "message": "No project loaded" })),
    };
    let auto_approve_tools = s.auto_approve_tools;
    if s.trace.run.state().is_terminal() {
        s.trace = ExecutionTrace::new();
    }
    s.worker_active = true;
    s.terminal_event_emitted = false;
    s.cancel_requested.store(false, Ordering::Release);
    s.trace.start(3);
    publish_event_locked(&mut s, UiEvent::AgentStateChanged(RunState::Starting));
    persist_snapshot_locked(&s);

    let state_ref = Arc::clone(&state);
    let final_state = Arc::clone(&state_ref);

    tokio::spawn(async move {
        let worker = async move {
            loop {
                if state_ref.lock().await.cancel_requested.load(Ordering::Acquire) {
                    let mut state = state_ref.lock().await;
                    state.trace.run.cancel(CancellationReason::Requested);
                    break;
                }

                let root_clone = root.clone();
                let p_config = planner_config();
                let e_config = executor_config();

                let mut trace;
                let prev_step_count;
                {
                    let state = state_ref.lock().await;
                    trace = state.trace.clone();
                    prev_step_count = trace.steps.len();
                }

                let run_res: Result<RunResult, magy_core::Error> = match tokio::time::timeout(
                    WORKER_OPERATION_TIMEOUT,
                    tokio::task::spawn_blocking(move || {
                        let planner = LmStudioProvider::new(p_config);
                        let executor = LmStudioProvider::new(e_config);
                        let verification_command = recommended_verification_command(&root_clone).unwrap_or_default();
                        let mut policy = DefaultApprovalPolicy::default()
                            .allow_command(verification_command.clone())
                            .auto_approve(auto_approve_tools);

                        // Allow common Rust/Node verification patterns if auto-approve is on
                        if auto_approve_tools {
                            policy = policy.allow_command("cargo run")
                                .allow_command("cargo run --release")
                                .allow_command("cargo test")
                                .allow_command("cargo check")
                                .allow_command("npm start")
                                .allow_command("npm test");
                        }

                        run_project_workflow(
                            root_clone,
                            &planner,
                            &executor,
                            &policy,
                            &verification_command,
                            SYSTEM_PROMPT,
                            1,
                            3,
                            &mut trace,
                        ).map(|res| (res, trace))
                    }),
                )
                .await
                {
                    Ok(Ok(Ok((run_result, updated_trace)))) => {
                        let mut s = state_ref.lock().await;
                        s.trace = updated_trace;
                        s.project = Some(run_result.project.clone());
                        s.agent = Some(run_result.agent.clone());
                        publish_event_locked(&mut s, UiEvent::ProjectLoaded(to_ui_project(&run_result.project)));
                        Ok(run_result)
                    }
                    Ok(Ok(Err(e))) => Err(e),
                    Ok(Err(_)) => Err(MagyError::Internal("Worker task failed".to_string())),
                    Err(_) => Err(MagyError::Internal("Worker timed out".to_string())),
                };

                match run_res {
                    Ok(result) => {
                        let mut s = state_ref.lock().await;
                        let new_steps: Vec<_> = s.trace.steps.iter().enumerate().skip(prev_step_count).map(|(i, step)| (i, step.clone())).collect();
                        for (i, step) in new_steps {
                            publish_event_locked(&mut s, UiEvent::Step {
                                step: serde_json::to_value(step).unwrap_or_default(),
                                index: i,
                            });
                        }

                        if result.project_completed {
                            s.worker_active = false;
                            publish_terminal_locked(&mut s);
                            persist_snapshot_locked(&s);
                            break;
                        }

                        if result.state == RunState::AwaitingApproval {
                            s.worker_active = false;
                            publish_event_locked(&mut s, UiEvent::AgentStateChanged(RunState::AwaitingApproval));
                            persist_snapshot_locked(&s);
                            break;
                        }

                        if result.state.is_terminal() || result.state == RunState::Stalled {
                            s.worker_active = false;
                            publish_event_locked(&mut s, UiEvent::AgentStateChanged(result.state.clone()));
                            publish_terminal_locked(&mut s);
                            persist_snapshot_locked(&s);
                            break;
                        }

                        publish_event_locked(&mut s, UiEvent::AgentStateChanged(result.state.clone()));
                        persist_snapshot_locked(&s);
                    }
                    Err(e) => {
                        publish_event(&state_ref, UiEvent::Error(format!("{:?}", e))).await;
                        mark_run_failed(&state_ref, format!("{:?}", e)).await;
                        break;
                    }
                }
            }
        };

        match AssertUnwindSafe(worker).catch_unwind().await {
            Ok(()) => finalize_worker(&final_state, None).await,
            Err(_) => {
                publish_event(&final_state, UiEvent::Error("Agent worker panicked".to_string())).await;
                finalize_worker(&final_state, Some("Agent worker panicked".to_string())).await;
            }
        }
    });

    Json(serde_json::json!({ "status": "success" }))
}

async fn finalize_worker(state: &SharedState, error: Option<String>) {
    let mut s = state.lock().await;
    s.worker_active = false;
    if let Some(msg) = error {
        if !s.trace.run.state().is_terminal() {
            s.trace.run.fail(FailureReason::Internal(msg));
        }
    }
    publish_terminal_locked(&mut s);
    persist_snapshot_locked(&s);
}

async fn get_events(
    State(state): State<SharedState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<EventResponse> {
    let after_seq = params.get("after").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let s = state.lock().await;
    let events = s.event_history.iter().filter(|e| e.seq > after_seq).cloned().collect();
    Json(EventResponse { events })
}

#[derive(Serialize)]
struct EventResponse {
    events: Vec<UiEventEnvelope>,
}

#[derive(Deserialize)]
struct ResolveActionRequest {
    index: usize,
    approved: bool,
}

async fn resolve_action(
    State(state): State<SharedState>,
    Json(payload): Json<ResolveActionRequest>,
) -> Json<serde_json::Value> {
    let mut s = state.lock().await;
    let agent = s.agent.clone().unwrap_or_else(Agent::new);
    let res = resolve_pending_action(&agent, &mut s.trace, payload.index, payload.approved);

    match res {
        Ok(()) => {
            persist_snapshot_locked(&s);
            Json(serde_json::json!({ "status": "success" }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("{:?}", e) })),
    }
}

#[derive(Deserialize)]
struct SettingsRequest {
    auto_approve_tools: bool,
}

async fn update_settings(
    State(state): State<SharedState>,
    Json(payload): Json<SettingsRequest>,
) -> Json<serde_json::Value> {
    let mut s = state.lock().await;
    s.auto_approve_tools = payload.auto_approve_tools;
    Json(serde_json::json!({ "status": "success" }))
}

async fn github_info(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let root = state.lock().await.root.clone();
    let Some(root) = root else {
        return Json(serde_json::json!({ "status": "error", "message": "No project loaded" }));
    };

    let result = tokio::task::spawn_blocking(move || {
        let output = |args: &[&str]| Command::new("git").args(args).current_dir(&root).output();
        let remote = output(&["config", "--get", "remote.origin.url"])
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string());
        let branch = output(&["branch", "--show-current"])
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string());
        let status = output(&["status", "--short"]);
        let changed_files = status.as_ref().ok().filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).lines().count()).unwrap_or(0);

        (remote, branch, status.is_ok(), changed_files)
    }).await.unwrap();

    let (remote, branch, is_git, changed_files) = result;
    Json(serde_json::json!({
        "status": "success",
        "github": {
            "is_git_repository": is_git,
            "branch": branch,
            "remote_url": remote,
            "github_url": remote.as_deref().and_then(github_remote_url),
            "changed_files": changed_files,
        }
    }))
}

fn github_remote_url(remote: &str) -> Option<String> {
    let trimmed = remote.trim_end_matches('/');
    let path = if let Some(value) = trimmed.strip_prefix("git@github.com:") { value }
    else if let Some(value) = trimmed.strip_prefix("https://github.com/") { value }
    else { return None; };
    Some(format!("https://github.com/{}", path.trim_end_matches(".git")))
}

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
}

async fn chat(
    State(state): State<SharedState>,
    Json(payload): Json<ChatRequest>,
) -> Json<serde_json::Value> {
    let (project, history) = {
        let s = state.lock().await;
        (s.project.clone(), s.chat_history.clone())
    };
    let Some(project) = project else {
        return Json(serde_json::json!({ "status": "error", "message": "Load a project before chatting" }));
    };

    let user_message = payload.message.clone();
    let result = tokio::task::spawn_blocking(move || {
        let provider = LmStudioProvider::new(executor_config());
        provider.ask_chat(&user_message, &history, Some(&project))
    }).await.unwrap();

    match result {
        Ok(reply) => {
            let mut s = state.lock().await;
            s.chat_history.push(("user".to_string(), payload.message));
            s.chat_history.push(("assistant".to_string(), reply.clone()));
            Json(serde_json::json!({ "status": "success", "reply": reply }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("{:?}", e) })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_events_are_emitted_once_with_monotonic_sequences() {
        let mut state = AppState {
            root: Some(PathBuf::from("/test")),
            project: Some(Project {
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
            }),
            agent: None,
            trace: ExecutionTrace::new(),
            worker_active: false,
            cancel_requested: Arc::new(AtomicBool::new(false)),
            event_seq: 0,
            event_history: VecDeque::new(),
            terminal_event_emitted: false,
            auto_approve_tools: false,
            chat_history: Vec::new(),
        };
        state.trace.start(1);
        state.trace.run.complete(vec![]);
        publish_terminal_locked(&mut state);
        assert!(state.terminal_event_emitted);
        assert_eq!(state.event_history.len(), 1);
        publish_terminal_locked(&mut state);
        assert_eq!(state.event_history.len(), 1);
    }
}
