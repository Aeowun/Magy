// Copyright (C) 2026 Zachary Joubert
use axum::{
    extract::State,
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use futures_util::stream::Stream;
use magy_core::{
    assemble_project_context, open_project, plan_execution, recommended_verification_command,
    resolve_pending_action, run_execution_cycle, select_task, DefaultApprovalPolicy,
    ExecutionTrace, LmStudioConfig, LmStudioProvider, Project,
};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, Mutex};
use tokio_stream::StreamExt;
use tower_http::services::ServeDir;

const SYSTEM_PROMPT: &str = r#"You are Magy, a privacy-first software engineering agent operating inside an existing user-selected project.

Return exactly one JSON object matching the tool schema. Use only the listed tools.
Never invent tools such as git, bash, shell, powershell, or terminal.

Do not initialize Git or create a repository. Do not install packages, access the network,
change permissions, delete files, or perform unrelated setup unless the active task explicitly
requires it. Never infer that repository setup is needed.

Use run_command only when it directly advances the active task and is an allowlisted
verification/build command. Prefer read_file, write_file, and list_directory for project work.
If a command is denied, choose a different action instead of repeating it.

All fields (tool, path, content, command) are required. Use null for fields that do not apply.
Only use task_complete after the requested work has actually been performed and
no required action was denied or failed. A model assertion is not verification.
All tool paths are relative to the selected project root; use "index.html", not
"/index.html". If generic verification is unavailable, report that limitation
instead of claiming success."#;

#[derive(Clone, Serialize, Deserialize, Debug)]
struct UiProject {
    name: String,
    goal: String,
    tasks: Vec<UiTask>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct UiTask {
    id: String,
    description: String,
    status: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
enum UiEvent {
    ProjectLoaded(UiProject),
    AgentStateChanged(String),
    Step {
        step: serde_json::Value,
        index: usize,
    },
    Stop(String),
    Error(String),
    Warning(String),
    Chat {
        role: String,
        content: String,
    },
}

struct AppState {
    root: Option<PathBuf>,
    project: Option<Project>,
    trace: ExecutionTrace,
    event_tx: mpsc::UnboundedSender<UiEvent>,
    auto_approve_tools: bool,
    chat_history: Vec<(String, String)>,
}

type SharedState = Arc<Mutex<AppState>>;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let (event_tx, _) = mpsc::unbounded_channel();

    let state = Arc::new(Mutex::new(AppState {
        root: None,
        project: None,
        trace: ExecutionTrace::new(),
        event_tx,
        auto_approve_tools: false,
        chat_history: Vec::new(),
    }));

    let ui_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("magy-app must live in the workspace")
        .join("magy-ui");
    let app = Router::new()
        .route("/api/load-project", post(load_project))
        .route("/api/initialize", post(initialize_project))
        .route("/api/run", post(run_agent))
        .route("/api/chat", post(chat))
        .route("/api/resolve", post(resolve_action))
        .route("/api/settings", post(update_settings))
        .route("/api/events", get(events_handler))
        .fallback_service(ServeDir::new(ui_dir))
        .with_state(state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Magy UI started at http://{}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn load_project(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let folder = rfd::FileDialog::new().pick_folder();
    if let Some(root) = folder {
        let root_clone = root.clone();
        let res = tokio::task::spawn_blocking(move || open_project(root_clone))
            .await
            .unwrap();

        match res {
            Ok((_, project)) => {
                let mut s = state.lock().await;
                s.root = Some(root);
                s.project = Some(project.clone());
                s.trace = ExecutionTrace::new();
                let ui_project = to_ui_project(&project);
                s.event_tx
                    .send(UiEvent::ProjectLoaded(ui_project.clone()))
                    .ok();
                return Json(serde_json::json!({ "status": "success", "project": ui_project }));
            }
            Err(e) => {
                let mut s = state.lock().await;
                s.root = Some(root);
                return Json(
                    serde_json::json!({ "status": "error", "message": format!("{:?}", e) }),
                );
            }
        }
    }
    Json(serde_json::json!({ "status": "cancelled" }))
}

#[derive(Deserialize)]
struct InitRequest {
    goal: String,
}

async fn initialize_project(
    State(state): State<SharedState>,
    Json(payload): Json<InitRequest>,
) -> Json<serde_json::Value> {
    let mut s = state.lock().await;
    let root = s.root.clone().ok_or("No directory selected").unwrap();

    let config = LmStudioConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        model_name: "nvidia/nemotron-3-nano-4b".to_string(),
    };

    let goal = payload.goal;
    let res = tokio::task::spawn_blocking(move || {
        let provider = LmStudioProvider::new(config);
        magy_core::initialize_project(root, &provider, &goal)
    })
    .await
    .unwrap();

    match res {
        Ok(project) => {
            s.project = Some(project.clone());
            let ui_project = to_ui_project(&project);
            s.event_tx
                .send(UiEvent::ProjectLoaded(ui_project.clone()))
                .ok();
            Json(serde_json::json!({ "status": "success", "project": ui_project }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("{:?}", e) })),
    }
}

async fn run_agent(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let s = state.lock().await;
    let root = s.root.clone().ok_or("No project loaded").unwrap();
    let event_tx = s.event_tx.clone();
    let auto_approve_tools = s.auto_approve_tools;

    let state_ref = Arc::clone(&state);

    tokio::spawn(async move {
        let res = tokio::task::spawn_blocking(move || open_project(root))
            .await
            .unwrap();
        if res.is_err() {
            return;
        }

        let (mut agent, mut project) = res.unwrap();

        loop {
            let root_path = agent.root().unwrap().to_path_buf();
            let config = LmStudioConfig {
                base_url: "http://localhost:1234/v1".to_string(),
                model_name: "nvidia/nemotron-3-nano-4b".to_string(),
            };

            let mut trace;
            let prev_step_count;
            {
                let s_lock = state_ref.lock().await;
                trace = s_lock.trace.clone();
                prev_step_count = trace.steps.len();
            }

            let run_res = tokio::task::spawn_blocking(move || {
                let verification_command =
                    recommended_verification_command(&root_path).unwrap_or_default();
                let context = assemble_project_context(root_path, project.clone()).unwrap();
                let plan = plan_execution(&agent, &context).unwrap();

                let next_task = match plan.tasks.first() {
                    Some(t) => t.clone(),
                    None => return (None, agent, project, trace),
                };

                select_task(&mut agent, &project, &next_task.id).unwrap();

                let provider = LmStudioProvider::new(config);
                let policy = DefaultApprovalPolicy::default()
                    .allow_command(verification_command.clone())
                    .auto_approve(auto_approve_tools);
                let res = run_execution_cycle(
                    &mut agent,
                    &mut project,
                    &context,
                    &plan,
                    &provider,
                    &policy,
                    SYSTEM_PROMPT,
                    10,
                    3,
                    &verification_command,
                    &mut trace,
                );
                (Some(res), agent, project, trace)
            })
            .await
            .unwrap();

            let (cycle_res, agent_new, project_new, trace_new) = run_res;
            agent = agent_new;
            project = project_new;
            trace = trace_new;

            {
                let mut s_lock = state_ref.lock().await;
                s_lock.trace = trace.clone();
                s_lock.project = Some(project.clone());
            }

            for (i, step) in trace.steps.iter().enumerate().skip(prev_step_count) {
                if let Some(verification) = &step.verification {
                    if !verification.passed {
                        event_tx
                            .send(UiEvent::Warning(format!(
                                "{} failed (exit code {:?}). Magy will use the output to continue.",
                                verification.command, verification.exit_code
                            )))
                            .ok();
                    }
                }
                event_tx
                    .send(UiEvent::Step {
                        step: serde_json::to_value(step).unwrap(),
                        index: i,
                    })
                    .ok();
            }

            let stop_reason = trace.stopped_reason.clone();
            event_tx.send(UiEvent::Stop(stop_reason.clone())).ok();

            if stop_reason == "Action requires approval" {
                break;
            }

            if stop_reason.starts_with("Runtime error")
                || stop_reason == "Tool execution failed"
                || stop_reason == "Model stopped without action"
                || stop_reason == "Maximum steps reached"
                || stop_reason.starts_with("Verification warning")
                || stop_reason.starts_with("Task completion rejected")
            {
                break;
            }

            event_tx
                .send(UiEvent::ProjectLoaded(to_ui_project(&project)))
                .ok();

            if let Some(Err(e)) = cycle_res {
                event_tx.send(UiEvent::Error(format!("{:?}", e))).ok();
                break;
            }
            if cycle_res.is_none() {
                break; // No more tasks
            }
        }
    });

    Json(serde_json::json!({ "status": "started" }))
}

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
}

async fn chat(
    State(state): State<SharedState>,
    Json(payload): Json<ChatRequest>,
) -> Json<serde_json::Value> {
    let message = payload.message.trim().to_string();
    if message.is_empty() {
        return Json(serde_json::json!({ "status": "error", "message": "Message is empty" }));
    }
    let (project, history, event_tx) = {
        let s = state.lock().await;
        (
            s.project.clone(),
            s.chat_history.clone(),
            s.event_tx.clone(),
        )
    };
    let Some(project) = project else {
        return Json(
            serde_json::json!({ "status": "error", "message": "Load a project before chatting" }),
        );
    };
    let message_for_model = message.clone();
    let result = tokio::task::spawn_blocking(move || {
        let provider = LmStudioProvider::new(LmStudioConfig {
            base_url: "http://localhost:1234/v1".to_string(),
            model_name: "nvidia/nemotron-3-nano-4b".to_string(),
        });
        let _ = project;
        provider.ask_chat(&message_for_model, &history)
    })
    .await
    .unwrap();

    match result {
        Ok(reply) => {
            let mut s = state.lock().await;
            s.chat_history.push(("user".to_string(), message.clone()));
            s.chat_history
                .push(("assistant".to_string(), reply.clone()));
            event_tx
                .send(UiEvent::Chat {
                    role: "user".to_string(),
                    content: message,
                })
                .ok();
            event_tx
                .send(UiEvent::Chat {
                    role: "assistant".to_string(),
                    content: reply.clone(),
                })
                .ok();
            Json(serde_json::json!({ "status": "success", "reply": reply }))
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
    let status = if s.auto_approve_tools {
        "Auto-approve enabled"
    } else {
        "Approval required"
    };
    s.event_tx
        .send(UiEvent::AgentStateChanged(status.to_string()))
        .ok();
    Json(serde_json::json!({
        "status": "success",
        "auto_approve_tools": s.auto_approve_tools
    }))
}

#[derive(Deserialize)]
struct ResolveRequest {
    index: usize,
    approved: bool,
}

async fn resolve_action(
    State(state): State<SharedState>,
    Json(payload): Json<ResolveRequest>,
) -> Json<serde_json::Value> {
    let s_lock = state.lock().await;
    let root = s_lock.root.clone().unwrap();
    let mut trace = s_lock.trace.clone();
    let index = payload.index;
    let approved = payload.approved;
    let event_tx = s_lock.event_tx.clone();
    drop(s_lock);

    let res = tokio::task::spawn_blocking(move || {
        let result = (|| {
            let (mut agent, project) = open_project(root).map_err(|e| format!("{:?}", e))?;
            let task_id = trace
                .steps
                .get(index)
                .and_then(|step| step.action_record.as_ref())
                .map(|record| record.task_id.clone())
                .ok_or_else(|| "Pending action record was not found".to_string())?;
            select_task(&mut agent, &project, &task_id).map_err(|e| format!("{:?}", e))?;
            resolve_pending_action(&agent, &mut trace, index, approved)
                .map_err(|e| format!("{:?}", e))
        })();
        (result, trace)
    })
    .await
    .unwrap();

    let (resolve_res, trace_new) = res;

    if let Err(message) = &resolve_res {
        event_tx.send(UiEvent::Error(message.clone())).ok();
        return Json(serde_json::json!({ "status": "error", "message": message }));
    }

    {
        let mut s_lock = state.lock().await;
        s_lock.trace = trace_new.clone();
        let step = &s_lock.trace.steps[payload.index];
        event_tx
            .send(UiEvent::Step {
                step: serde_json::to_value(step).unwrap(),
                index: payload.index,
            })
            .ok();
    }

    Json(serde_json::json!({ "status": "success" }))
}

async fn events_handler(
    State(state): State<SharedState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::unbounded_channel();
    {
        let mut s = state.lock().await;
        s.event_tx = tx;
    }

    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx)
        .map(|event| Event::default().data(serde_json::to_string(&event).unwrap()))
        .map(Ok);

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::new())
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
