use super::state::IRIS_FAILED_PID;
use crate::intercept::{iris, set_active};
use http::{BridgeHttpResponse, forward_iris_endpoint_http, forward_iris_http};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::Ordering,
    thread,
    time::Duration,
};
use tracing::{error, info, warn};
mod http;
pub(super) fn launch_iris_bridge(
    listener: TcpListener,
    token: String,
    bridge_url: String,
    endpoint_bridge_url: String,
    endpoint_prefix: String,
) {
    if let Err(error) = thread::Builder::new()
        .name("noa-iris-bridge".to_string())
        .spawn(move || {
            iris_bridge_loop(
                listener,
                token,
                bridge_url,
                endpoint_bridge_url,
                endpoint_prefix,
            )
        })
    {
        error!(%error, "Iris 네이티브 브리지 스레드를 시작하지 못했습니다");
    }
}

fn iris_bridge_loop(
    listener: TcpListener,
    token: String,
    bridge_url: String,
    endpoint_bridge_url: String,
    endpoint_prefix: String,
) {
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let token = token.clone();
                let bridge_url = bridge_url.clone();
                let endpoint_bridge_url = endpoint_bridge_url.clone();
                let endpoint_prefix = endpoint_prefix.clone();
                let _ = thread::Builder::new()
                    .name("noa-iris-request".to_string())
                    .spawn(move || {
                        if let Err(message) = handle_iris_connection(
                            stream,
                            &token,
                            &bridge_url,
                            &endpoint_bridge_url,
                            &endpoint_prefix,
                        ) {
                            warn!(error = %message, "Iris Rust 에이전트 요청 실패");
                        }
                    });
            }
            Err(error) => warn!(%error, "Iris 네이티브 연결 수락 실패"),
        }
    }
}

fn handle_iris_connection(
    mut stream: TcpStream,
    token: &str,
    bridge_url: &str,
    endpoint_bridge_url: &str,
    endpoint_prefix: &str,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(125)))
        .map_err(|error| error.to_string())?;
    stream
        .set_nodelay(true)
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err("Iris 에이전트 요청이 비어 있습니다".to_string());
    }
    let request: Value = serde_json::from_str(line.trim()).map_err(|error| error.to_string())?;
    if request.get("token").and_then(Value::as_str) != Some(token) {
        write_iris_response(&mut stream, None, Err("authentication failed".to_string()))?;
        return Ok(());
    }
    let id = request.get("id").and_then(Value::as_u64);
    match request.get("event").and_then(Value::as_str) {
        Some("ready") => {
            let pid = request
                .get("pid")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            IRIS_FAILED_PID.store(0, Ordering::Release);
            set_active(true);
            info!(pid, "Iris Rust 에이전트 준비 완료");
            write_iris_response(&mut stream, None, Ok(()))?;
        }
        Some("error") => {
            let pid = request
                .get("pid")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let detail = request
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("알 수 없는 초기화 오류");
            if let Ok(pid) = u32::try_from(pid) {
                IRIS_FAILED_PID.store(pid, Ordering::Release);
            }
            set_active(false);
            warn!(pid, error = detail, "Iris Rust 에이전트 초기화 실패");
            write_iris_response(&mut stream, None, Ok(()))?;
        }
        Some("reply") => {
            let payload = request
                .get("payload")
                .and_then(Value::as_str)
                .ok_or_else(|| "Iris reply payload가 없습니다".to_string())?;
            let reply_type = request
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let room = request
                .get("room")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let result = forward_iris_http(bridge_url, token, payload);
            if result.is_ok() {
                info!(reply_type, room, "Iris /reply 선택 처리");
            }
            write_iris_response(&mut stream, id, result)?;
        }
        Some("endpoint") => {
            let method = request
                .get("method")
                .and_then(Value::as_str)
                .ok_or_else(|| "Iris endpoint method가 없습니다".to_string())?;
            let uri = request
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| "Iris endpoint URI가 없습니다".to_string())?;
            let content_type = request
                .get("contentType")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream");
            let body = iris::endpoint_body(&request)?;
            let result = forward_iris_endpoint_http(
                endpoint_bridge_url,
                endpoint_prefix,
                token,
                method,
                uri,
                content_type,
                &body,
            );
            write_iris_endpoint_response(&mut stream, id, result)?;
        }
        _ => write_iris_response(&mut stream, id, Err("unknown Iris event".to_string()))?,
    }
    Ok(())
}

fn write_iris_endpoint_response(
    stream: &mut TcpStream,
    id: Option<u64>,
    result: Result<BridgeHttpResponse, String>,
) -> Result<(), String> {
    let response = match result {
        Ok(response) => serde_json::json!({
            "id": id,
            "ok": true,
            "status": response.status,
            "contentType": response.content_type,
            "body": response.body,
        }),
        Err(message) => serde_json::json!({"id": id, "ok": false, "error": message}),
    };
    serde_json::to_writer(&mut *stream, &response).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    Ok(())
}

fn write_iris_response(
    stream: &mut TcpStream,
    id: Option<u64>,
    result: Result<(), String>,
) -> Result<(), String> {
    let response = match result {
        Ok(()) => serde_json::json!({"id": id, "ok": true}),
        Err(message) => serde_json::json!({"id": id, "ok": false, "error": message}),
    };
    serde_json::to_writer(&mut *stream, &response).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    Ok(())
}
