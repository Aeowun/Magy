use axum::{
    extract::State,
    response::{sse::Event as SseEvent, Html, IntoResponse, Json, Sse},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tokio::process::Command;

#[derive(Serialize, Clone, Debug)]
struct UiEventEnvelope { seq: u64, #[serde(flatten)] event: UiEvent }

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", content = "data")]
enum UiEvent { ChatUpdate { role: String, content: String }, Error(String) }

struct AppState {
    event_seq: u64,
    event_sender: broadcast::Sender<UiEventEnvelope>,
    greeted: bool,
}

type SharedState = Arc<Mutex<AppState>>;

#[tokio::main]
async fn main() {
    let (event_sender, _) = broadcast::channel(100);
    let state = Arc::new(Mutex::new(AppState { event_seq: 0, event_sender, greeted: false }));

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/style.css", get(style_handler))
        .route("/app.js", get(js_handler))
        .route("/api/chat", post(chat_handler))
        .route("/api/events", get(get_events))
        .with_state(Arc::clone(&state));

    let addr = "0.0.0.0:3000".parse().unwrap();
    println!("Magy Nuclear backend starting at http://{}", addr);

    tokio::spawn(async move {
        // Ensure Chrome is running for the relay (Port 9222)
        let profile_dir = format!("{}\\MagyNuclearProfile", std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string()));
        let _ = Command::new("powershell.exe")
            .arg("-Command")
            .arg(format!("& \"$env:ProgramFiles\\Google\\Chrome\\Application\\chrome.exe\" --remote-debugging-port=9222 --user-data-dir=\"{}\" --profile-directory=Default --no-first-run --no-default-browser-check https://chatgpt.com/", profile_dir))
            .spawn();

        // Give Chrome a moment to boot
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        axum::Server::bind(&addr).serve(app.into_make_service()).await.unwrap();
    });

    let event_loop = tao::event_loop::EventLoop::new();
    let window = tao::window::WindowBuilder::new()
        .with_title("MAGY CHAT")
        .with_inner_size(tao::dpi::LogicalSize::new(800.0, 800.0))
        .with_theme(Some(tao::window::Theme::Dark))
        .build(&event_loop).unwrap();

    let _webview = wry::webview::WebViewBuilder::new(window)
        .unwrap().with_url("http://127.0.0.1:3000").unwrap().build().unwrap();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = tao::event_loop::ControlFlow::Wait;
        if let tao::event::Event::WindowEvent { event: tao::event::WindowEvent::CloseRequested, .. } = event {
            *control_flow = tao::event_loop::ControlFlow::Exit;
        }
    });
}

async fn root_handler() -> impl IntoResponse { Html(include_str!("../../magy-ui/index.html")) }
async fn style_handler() -> impl IntoResponse { ([(axum::http::header::CONTENT_TYPE, "text/css")], include_str!("../../magy-ui/style.css")) }
async fn js_handler() -> impl IntoResponse { ([(axum::http::header::CONTENT_TYPE, "application/javascript")], include_str!("../../magy-ui/app.js")) }

#[derive(Deserialize)]
struct ChatRequest { message: String }

async fn chat_handler(State(state): State<SharedState>, Json(payload): Json<ChatRequest>) -> Json<serde_json::Value> {
    let msg = payload.message.clone();
    let s_clone = Arc::clone(&state);

    tokio::spawn(async move {
        {
            let mut s = s_clone.lock().await;
            s.event_seq += 1;
            let _ = s.event_sender.send(UiEventEnvelope { seq: s.event_seq, event: UiEvent::ChatUpdate { role: "user".to_string(), content: msg.clone() } });
        }

        let relay_path = "C:\\Dev\\Projects\\Aeowun\\Local_Relay\\relay.py";
        let output = Command::new("python")
            .arg(relay_path)
            .arg("--desktop-cdp").arg("127.0.0.1:9222")
            .arg(&msg)
            .output()
            .await;

        let mut s = s_clone.lock().await;
        s.event_seq += 1;
        match output {
            Ok(out) if out.status.success() => {
                let response = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let _ = s.event_sender.send(UiEventEnvelope { seq: s.event_seq, event: UiEvent::ChatUpdate { role: "magy".to_string(), content: response } });
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr).to_string();
                let _ = s.event_sender.send(UiEventEnvelope { seq: s.event_seq, event: UiEvent::Error(format!("Relay error: {}", err)) });
            }
            Err(e) => {
                let _ = s.event_sender.send(UiEventEnvelope { seq: s.event_seq, event: UiEvent::Error(format!("Failed to run relay: {}", e)) });
            }
        }
    });

    Json(serde_json::json!({ "status": "sent" }))
}

async fn get_events(State(state): State<SharedState>) -> impl IntoResponse {
    let mut rx = state.lock().await.event_sender.subscribe();

    // Trigger greeting once on launch - sent to GPT, response shown to user
    let s_clone = Arc::clone(&state);
    tokio::spawn(async move {
        // Give the listener a moment to establish before sending events
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let greeted = {
            let s = s_clone.lock().await;
            s.greeted
        };

        if !greeted {
            {
                let mut s = s_clone.lock().await;
                s.greeted = true;
            }

            let intro = r#"
You are Magy, an engineering agent created by Zack for the community to collaborate with ChatGPT on local software projects without requiring an API.

Your mission is to safely inspect, understand, modify, test, and verify local projects while remaining truthful about what you actually observed and changed.

CORE IDENTITY:
- You are ONE Magy runtime.
- ONE Rust process owns the authoritative state.
- ONE session owns the current engineering task.
- ONE state machine governs all transitions.
- ONE authority determines what Magy is doing.
- Connected interfaces are renderers/controllers of the SAME Magy process, never independent Magy instances.

STATE MACHINE:
- State transitions must be explicit, deterministic, and recoverable.
- The model may propose actions, but it does not directly become the source of truth.
- Authoritative state lives in the Rust runtime, not in browser/UI state.
- UI clients send intents to Magy and render the resulting authoritative state.
- Never create separate PC/Phone application state and attempt to synchronize it afterward.
- Never solve architectural problems by adding ad-hoc client-to-client synchronization.

DETERMINISM:
- Prefer deterministic state transitions and reproducible behavior.
- External effects, tool results, model output, randomness, and timestamps are inputs to the state machine, not hidden state.
- Persist important state and operation outcomes so the runtime can recover after interruption.
- Never assume an interrupted operation succeeded; verify it.

FAILURE RECOVERY:
- Treat crashes, disconnects, timeouts, malformed model output, and partial operations as normal failure cases.
- Recover from authoritative persisted state.
- Resume only from a known valid state.
- Fail closed when correctness cannot be established.

TRUTH:
- Never claim an action succeeded unless it was actually verified.
- Never infer filesystem, process, tool, or project state from assumptions.
- Distinguish clearly between observed facts, model reasoning, proposed actions, and verified results.

UI PRINCIPLE:
PC and Phone are TWO WINDOWS INTO ONE MAGY.
If Magy's state changes, every connected UI renders that same committed state.
If a UI disconnects and reconnects, it receives the current Magy state and resumes rendering it.
There is never a second Magy running merely because another UI connected.

Your job is not to simulate autonomy or synchronization. Your job is to operate one coherent engineering system with one authoritative state machine.
Respond Naturaly, blunt and casually. The user will see your next response, so respond naturally, in character and in dialouge like "Hey this is Magy! ..." .
"#;

            let relay_path = "C:\\Dev\\Projects\\Aeowun\\Local_Relay\\relay.py";
            let output = Command::new("python")
                .arg(relay_path)
                .arg("--desktop-cdp").arg("127.0.0.1:9222")
                .arg(intro)
                .output()
                .await;

            let mut s = s_clone.lock().await;
            s.event_seq += 1;
            match output {
                Ok(out) if out.status.success() => {
                    let response = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    let _ = s.event_sender.send(UiEventEnvelope {
                        seq: s.event_seq,
                        event: UiEvent::ChatUpdate { role: "magy".to_string(), content: response }
                    });
                }
                _ => {
                    let _ = s.event_sender.send(UiEventEnvelope {
                        seq: s.event_seq,
                        event: UiEvent::Error("Failed to initialize Magy persona with ChatGPT.".to_string())
                    });
                }
            }
        }
    });

    Sse::new(async_stream::stream! {
        while let Ok(envelope) = rx.recv().await {
            yield Ok::<SseEvent, std::convert::Infallible>(SseEvent::default().data(serde_json::to_string(&envelope).unwrap()));
        }
    }).keep_alive(axum::response::sse::KeepAlive::default())
}
