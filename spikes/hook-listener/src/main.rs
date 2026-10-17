//! Spike S1/S3/S4: log every hook request (headers + body) to `captured/`, and answer
//! PermissionRequest according to a mode set via `GET /mode?m=...`:
//!   none (default) -> `{}`;  allow;  deny;  hold:<secs>[:allow|deny|none] -> sleep then answer.
//! Usage: hook-listener <bind-ip> [<bind-ip> ...]   (port 47821)
use axum::{
    Router,
    body::Bytes,
    extract::{Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct App {
    seq: AtomicU64,
    mode: Mutex<String>,
    started: Instant,
}

fn decision(behavior: &str) -> Value {
    let mut d = json!({ "behavior": behavior });
    if behavior == "deny" {
        d["message"] = json!("Denied from cc-sidebar spike");
    }
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": d } })
}

async fn hook(State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> String {
    let n = app.seq.fetch_add(1, Ordering::SeqCst);
    let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let event = payload["hook_event_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let hdrs: serde_json::Map<String, Value> = headers
        .iter()
        .filter(|(k, _)| k.as_str().starts_with("x-"))
        .map(|(k, v)| (k.to_string(), json!(v.to_str().unwrap_or(""))))
        .collect();
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let mut rec = payload.clone();
    if let Value::Object(m) = &mut rec {
        m.insert("_headers".into(), Value::Object(hdrs));
        m.insert("_received_ms".into(), json!(ms));
    }
    let _ = std::fs::create_dir_all("captured");
    let _ = std::fs::write(
        format!("captured/{n:04}-{event}.json"),
        serde_json::to_string_pretty(&rec).unwrap(),
    );
    println!(
        "[{:>7.2}s] #{n} {event} {}",
        app.started.elapsed().as_secs_f32(),
        payload["tool_name"].as_str().unwrap_or("")
    );

    if event != "PermissionRequest" {
        return String::new();
    }
    let mode = app.mode.lock().unwrap().clone();
    let mut parts = mode.split(':');
    let answer = match parts.next().unwrap_or("none") {
        "allow" => Some("allow"),
        "deny" => Some("deny"),
        "hold" => {
            let secs: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(30);
            println!("    holding {secs}s");
            tokio::time::sleep(Duration::from_secs(secs)).await;
            match parts.next() {
                Some("allow") => Some("allow"),
                Some("deny") => Some("deny"),
                _ => None,
            }
        }
        _ => None,
    };
    let out = answer.map(decision).unwrap_or(json!({}));
    println!("    -> {out}");
    out.to_string()
}

async fn set_mode(State(app): State<Arc<App>>, Query(q): Query<HashMap<String, String>>) -> String {
    let m = q.get("m").cloned().unwrap_or_else(|| "none".into());
    *app.mode.lock().unwrap() = m.clone();
    println!("mode = {m}");
    format!("mode={m}\n")
}

#[tokio::main]
async fn main() {
    let app = Arc::new(App {
        seq: AtomicU64::new(0),
        mode: Mutex::new("none".into()),
        started: Instant::now(),
    });
    let router = Router::new()
        .route("/hook", post(hook))
        .route("/mode", get(set_mode))
        .with_state(app);
    let ips: Vec<String> = std::env::args().skip(1).collect();
    let mut tasks = vec![];
    for ip in if ips.is_empty() {
        vec!["127.0.0.1".into()]
    } else {
        ips
    } {
        let l = tokio::net::TcpListener::bind(format!("{ip}:47821"))
            .await
            .expect("bind");
        println!("listening on {ip}:47821");
        let r = router.clone();
        tasks.push(tokio::spawn(
            async move { axum::serve(l, r).await.unwrap() },
        ));
    }
    for t in tasks {
        let _ = t.await;
    }
}
