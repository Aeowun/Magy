// Copyright (C) 2026 Zachary Joubert
use axum::{
    extract::State,
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use futures_util::stream::Stream;
use magy_core::{
    assemble_project_context, open_project, plan_execution, resolve_pending_action,
    run_execution_cycle, select_task, DefaultApprovalPolicy, ExecutionTrace,
    LmStudioConfig, LmStudioProvider, Project,
};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, path::PathBuf, sync::Arc};
use tokio::sync::{mpsc, Mutex};
use tokio_stream::StreamExt;
use tower_http::services::ServeDir;

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
    Step { step: serde_json::Value, index: usize },
    Stop(String),
    Error(String),
}

struct AppState {
    root: Option<PathBuf>,
    project: Option<Project>,
    trace: ExecutionTrace,
    event_tx: mpsc::UnboundedSender<UiEvent>,
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
    }));

    let app = Router::new()
        .route("/api/load-project", post(load_project))
        .route("/api/initialize", post(initialize_project))
        .route("/api/run", post(run_agent))
        .route("/api/resolve", post(resolve_action))
        .route("/api/events", get(events_handler))
        .fallback_service(ServeDir::new("magy-ui"))
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
        let res = tokio::task::spawn_blocking(move || open_project(root_clone)).await.unwrap();

        match res {
            Ok((_, project)) => {
                let mut s = state.lock().await;
                s.root = Some(root);
                s.project = Some(project.clone());
                s.trace = ExecutionTrace::new();
                let ui_project = to_ui_project(&project);
                s.event_tx.send(UiEvent::ProjectLoaded(ui_project.clone())).ok();
                return Json(serde_json::json!({ "status": "success", "project": ui_project }));
            }
            Err(e) => {
                let mut s = state.lock().await;
                s.root = Some(root);
                return Json(serde_json::json!({ "status": "error", "message": format!("{:?}", e) }));
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
    }).await.unwrap();

    match res {
        Ok(project) => {
            s.project = Some(project.clone());
            let ui_project = to_ui_project(&project);
            s.event_tx.send(UiEvent::ProjectLoaded(ui_project.clone())).ok();
            Json(serde_json::json!({ "status": "success", "project": ui_project }))
        }
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("{:?}", e) })),
    }
}

async fn run_agent(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let s = state.lock().await;
    let root = s.root.clone().ok_or("No project loaded").unwrap();
    let event_tx = s.event_tx.clone();

    let state_ref = Arc::clone(&state);

    tokio::spawn(async move {
        let res = tokio::task::spawn_blocking(move || open_project(root)).await.unwrap();
        if res.is_err() { return; }
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
                let context = assemble_project_context(root_path, project.clone()).unwrap();
                let plan = plan_execution(&agent, &context).unwrap();

                let next_task = match plan.tasks.first() {
                    Some(t) => t.clone(),
                    None => return (None, agent, project, trace),
                };

                select_task(&mut agent, &project, &next_task.id).unwrap();

                let provider = LmStudioProvider::new(config);
                let policy = DefaultApprovalPolicy;
                let res = run_execution_cycle(
                    &mut agent,
                    &mut project,
                    &context,
                    &plan,
                    &provider,
                    &policy,
                    "You are Magy, an autonomous AI agent. To interact with the project, you MUST output a single JSON code block matching the strict schema. All fields (tool, path, content, command) are REQUIRED. Use null if a field does not apply to the selected tool.",
                    10,
                    3,
                    "cargo test",
                    &mut trace,
                );
                (Some(res), agent, project, trace)
            }).await.unwrap();

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
                event_tx.send(UiEvent::Step {
                    step: serde_json::to_value(step).unwrap(),
                    index: i
                }).ok();
            }

            let stop_reason = trace.stopped_reason.clone();
            event_tx.send(UiEvent::Stop(stop_reason)).ok();

            if agent.state() != &magy_core::domain::agent::State::Planning {
                break;
            }

            event_tx.send(UiEvent::ProjectLoaded(to_ui_project(&project))).ok();

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
        let (agent, _) = open_project(root).unwrap();
        let res = resolve_pending_action(&agent, &mut trace, index, approved);
        (res, trace)
    }).await.unwrap();

    let (resolve_res, trace_new) = res;

    {
        let mut s_lock = state.lock().await;
        s_lock.trace = trace_new.clone();
        let step = &s_lock.trace.steps[payload.index];
        event_tx.send(UiEvent::Step {
            step: serde_json::to_value(step).unwrap(),
            index: payload.index
        }).ok();
    }

    match resolve_res {
        Ok(_) => Json(serde_json::json!({ "status": "success" })),
        Err(e) => Json(serde_json::json!({ "status": "error", "message": format!("{:?}", e) })),
    }
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
        .map(|event| {
            Event::default().data(serde_json::to_string(&event).unwrap())
        })
        .map(Ok);

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::new())
}

fn to_ui_project(project: &Project) -> UiProject {
    UiProject {
        name: project.name.clone(),
        goal: project.goal.clone(),
        tasks: project.tasks.iter().map(|t| UiTask {
            id: t.id.clone(),
            description: t.description.clone(),
            status: format!("{:?}", t.status),
        }).collect(),
    }
}
