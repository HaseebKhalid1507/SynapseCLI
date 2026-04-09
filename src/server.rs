use axum::{
    extract::Query,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::get,
    Router,
};
use agent_runtime::{Runtime, StreamEvent};
use futures::stream::StreamExt;
use std::collections::HashMap;
use tower_http::cors::CorsLayer;
use tracing_subscriber::fmt::init;

#[tokio::main]
async fn main() {
    init();

    let runtime = Runtime::new().await.expect("Failed to initialize runtime");

    let app = Router::new()
        .route("/", get(serve_html))
        .route("/chat", get(move |query| chat_stream(runtime, query)))
        .layer(CorsLayer::permissive());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
        
    println!("🚀 Server running on http://127.0.0.1:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn chat_stream(
    runtime: Runtime,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let prompt = match params.get("prompt").filter(|p| !p.is_empty()) {
        Some(p) => p.clone(),
        None => return (StatusCode::BAD_REQUEST, "Missing 'prompt' query parameter").into_response(),
    };

    let stream = runtime
        .run_stream(prompt)
        .filter_map(|event| async {
            let data = match event {
                StreamEvent::Thinking(text) => {
                    serde_json::json!({"type": "thinking", "text": text}).to_string()
                }
                StreamEvent::Text(text) => {
                    serde_json::json!({"type": "text", "text": text}).to_string()
                }
                StreamEvent::ToolUse { tool_name, tool_id, input } => {
                    serde_json::json!({"type": "tool_use", "tool_name": tool_name, "tool_id": tool_id, "input": input}).to_string()
                }
                StreamEvent::ToolResult { tool_id, result } => {
                    serde_json::json!({"type": "tool_result", "tool_id": tool_id, "result": result}).to_string()
                }
                StreamEvent::MessageHistory(_) => return None,
                StreamEvent::Done => {
                    r#"{"type":"done"}"#.to_string()
                }
                StreamEvent::Error(err) => {
                    serde_json::json!({"type": "error", "message": err}).to_string()
                }
            };

            Some(Ok::<_, std::convert::Infallible>(Event::default().data(data)))
        });

    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}


async fn serve_html() -> Html<&'static str> {
    Html(include_str!("../templates/index.html"))
}