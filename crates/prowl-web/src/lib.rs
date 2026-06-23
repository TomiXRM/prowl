//! prowl-web — Web(DOM) フロントエンド（方針A）。
//!
//! axum で HTTP+WebSocket サーバを立て、契約をブラウザへ橋渡しする：
//! - サーバ → ブラウザ: `AppState` を JSON でストリーム（変化のたび）
//! - ブラウザ → サーバ: `Command` を JSON で受信しエンジンへ
//!
//! GPUI と違い tokio とシームレスに繋がり、ブラウザは素の DOM なので
//! Playwright で end-to-end に検証できる（AIデバッグ向き）。

use std::net::Ipv4Addr;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use prowl_app::{AppState, Command, EngineHandle, Frontend};
use tokio::sync::{mpsc, watch};

const INDEX_HTML: &str = include_str!("index.html");

#[derive(Clone)]
struct Ctx {
    commands: mpsc::Sender<Command>,
    state: watch::Receiver<AppState>,
}

/// ブラウザを描画面とする Web フロントエンド。`http://127.0.0.1:<port>` を開く。
pub struct WebFrontend {
    port: u16,
}

impl WebFrontend {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}

#[async_trait::async_trait]
impl Frontend for WebFrontend {
    async fn run(self: Box<Self>, engine: EngineHandle) -> anyhow::Result<()> {
        let EngineHandle {
            commands, state, ..
        } = engine;
        let ctx = Ctx { commands, state };

        let app = Router::new()
            .route("/", get(index))
            .route("/ws", get(ws_handler))
            .with_state(ctx);

        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, self.port)).await?;
        println!("prowl web UI → http://127.0.0.1:{}", self.port);
        axum::serve(listener, app).await?;
        Ok(())
    }
}

async fn index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

async fn ws_handler(ws: WebSocketUpgrade, State(ctx): State<Ctx>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, ctx))
}

async fn handle_socket(socket: WebSocket, ctx: Ctx) {
    let (mut sender, mut receiver) = socket.split();
    let Ctx {
        commands,
        mut state,
    } = ctx;

    // サーバ → ブラウザ: 状態スナップショットを流す（初回 + 変化のたび）
    let mut send_task = tokio::spawn(async move {
        loop {
            let snapshot = state.borrow().clone();
            let Ok(txt) = serde_json::to_string(&snapshot) else {
                break;
            };
            if sender.send(Message::Text(txt)).await.is_err() {
                break;
            }
            if state.changed().await.is_err() {
                break; // エンジン終了
            }
        }
    });

    // ブラウザ → サーバ: コマンドを受け取りエンジンへ
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(t) = msg {
                if let Ok(cmd) = serde_json::from_str::<Command>(&t) {
                    let _ = commands.send(cmd).await;
                }
            }
        }
    });

    // どちらかが終わったら両方畳む
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}
