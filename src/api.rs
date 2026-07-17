use crate::{
    commands::tool_command,
    jobs::Scheduler,
    models::{Health, SubmitJob, ToolRequest},
};
use anyhow::Result as AnyResult;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, net::SocketAddr};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

pub async fn serve(address: SocketAddr, scheduler: Scheduler) -> AnyResult<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/api/jobs", post(submit).get(list))
        .route("/api/jobs/{id}", get(get_job))
        .route("/api/jobs/{id}/cancel", post(cancel))
        .route("/api/jobs/{id}/output", get(output))
        .route("/api/command", post(legacy_command))
        .route("/api/tools/{tool}", post(submit_tool))
        .layer(TraceLayer::new_for_http())
        .with_state(scheduler);
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "HTTP server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

#[derive(Debug)]
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(StatusCode::BAD_REQUEST, error.to_string())
    }
}

async fn submit(
    State(scheduler): State<Scheduler>,
    Json(request): Json<SubmitJob>,
) -> Result<impl IntoResponse, ApiError> {
    let job = scheduler.submit(request).await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

async fn submit_tool(
    State(scheduler): State<Scheduler>,
    Path(tool): Path<String>,
    Json(mut request): Json<ToolRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let timeout_seconds = take_u64(&mut request.values, "timeout_seconds")?;
    let webhook_url = take_string(&mut request.values, "webhook_url")?;
    let argv = tool_command(&tool, &request)?;
    let job = scheduler
        .submit(SubmitJob {
            tool: Some(tool),
            argv,
            timeout_seconds,
            webhook_url,
        })
        .await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

async fn legacy_command(
    State(scheduler): State<Scheduler>,
    Json(mut request): Json<ToolRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let timeout_seconds = take_u64(&mut request.values, "timeout_seconds")?;
    let webhook_url = take_string(&mut request.values, "webhook_url")?;
    let command = take_string(&mut request.values, "command")?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError(StatusCode::BAD_REQUEST, "command is required".into()))?;
    let argv = shell_words::split(&command)
        .map_err(|error| ApiError(StatusCode::BAD_REQUEST, error.to_string()))?;
    let job = scheduler
        .submit(SubmitJob {
            tool: Some("command".into()),
            argv,
            timeout_seconds,
            webhook_url,
        })
        .await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

fn take_u64(values: &mut BTreeMap<String, Value>, key: &str) -> Result<Option<u64>, ApiError> {
    match values.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(Some).ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                format!("{key} must be a positive integer"),
            )
        }),
        Some(_) => Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("{key} must be an integer"),
        )),
    }
}

fn take_string(
    values: &mut BTreeMap<String, Value>,
    key: &str,
) -> Result<Option<String>, ApiError> {
    match values.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("{key} must be a string"),
        )),
    }
}

async fn list(State(scheduler): State<Scheduler>) -> Json<Value> {
    Json(json!({"jobs": scheduler.list().await}))
}

async fn get_job(
    State(scheduler): State<Scheduler>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    scheduler
        .get(id)
        .await
        .map(|j| Json(json!(j)))
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "job not found".into()))
}

async fn cancel(
    State(scheduler): State<Scheduler>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    scheduler
        .cancel(id)
        .await
        .map(|j| Json(json!(j)))
        .map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))
}

#[derive(Deserialize)]
struct OutputQuery {
    #[serde(default = "stdout")]
    stream: String,
    #[serde(default)]
    offset: u64,
    #[serde(default = "output_limit")]
    limit: usize,
}
fn stdout() -> String {
    "stdout".into()
}
fn output_limit() -> usize {
    64 * 1024
}

async fn output(
    State(scheduler): State<Scheduler>,
    Path(id): Path<Uuid>,
    Query(query): Query<OutputQuery>,
) -> Result<Json<Value>, ApiError> {
    scheduler
        .output(id, &query.stream, query.offset, query.limit)
        .await
        .map(|o| Json(json!(o)))
        .map_err(ApiError::from)
}

async fn health(State(scheduler): State<Scheduler>) -> Json<Health> {
    let (queued, running, max_concurrency) = scheduler.counts().await;
    Json(Health {
        status: "healthy",
        service: "mcp-kali",
        queued,
        running,
        max_concurrency,
    })
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>MCP Kali Jobs</title><style>
:root{color-scheme:dark;font:15px system-ui;background:#0b1014;color:#dce7ec}body{max-width:1100px;margin:40px auto;padding:0 20px}h1{color:#67e8a5}button{background:#173b2c;color:#baffd8;border:1px solid #2f7657;padding:7px 11px;border-radius:5px;cursor:pointer}table{width:100%;border-collapse:collapse;margin-top:20px}th,td{text-align:left;padding:10px;border-bottom:1px solid #26343d}code{font-size:12px}.state{font-weight:650}.failed,.timed_out,.interrupted{color:#ff8888}.succeeded{color:#67e8a5}.running{color:#ffd166}pre{background:#111a20;padding:16px;overflow:auto;max-height:420px;white-space:pre-wrap}</style></head>
<body><h1>MCP Kali job monitor</h1><p>Durable, asynchronous command execution. This page refreshes every two seconds.</p><button onclick="load()">Refresh</button><table><thead><tr><th>State</th><th>Tool</th><th>Created</th><th>Job ID</th><th></th></tr></thead><tbody id="jobs"></tbody></table><pre id="output">Select a job to view stdout.</pre>
<script>const esc=s=>String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));async function load(){const r=await fetch('/api/jobs'),d=await r.json();jobs.innerHTML=d.jobs.map(j=>`<tr><td class="state ${j.state}">${esc(j.state)}</td><td>${esc(j.tool)}</td><td>${esc(j.created_at)}</td><td><code>${j.id}</code></td><td><button onclick="show('${j.id}')">Output</button></td></tr>`).join('')}async function show(id){const r=await fetch(`/api/jobs/${id}/output?limit=1048576`),d=await r.json();output.textContent=d.data||'(no stdout yet)'}load();setInterval(load,2000)</script></body></html>"#;
