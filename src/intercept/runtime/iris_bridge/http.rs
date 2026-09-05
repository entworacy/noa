use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};
const MAX_IRIS_HTTP_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
pub(super) fn forward_iris_http(url: &str, token: &str, payload: &str) -> Result<(), String> {
    let response = iris_http_transaction(
        url,
        token,
        "POST",
        "application/json; charset=utf-8",
        payload.as_bytes(),
    )?;
    if (200..300).contains(&response.status) {
        Ok(())
    } else {
        Err(format!(
            "Noa bridge returned HTTP {}{}",
            response.status,
            if response.body.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", response.body.trim())
            }
        ))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct BridgeHttpResponse {
    pub(super) status: u16,
    pub(super) content_type: String,
    pub(super) body: String,
}

pub(super) fn forward_iris_endpoint_http(
    bridge_url: &str,
    prefix: &str,
    token: &str,
    method: &str,
    uri: &str,
    content_type: &str,
    body: &[u8],
) -> Result<BridgeHttpResponse, String> {
    if !matches!(
        method,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" | "HEAD"
    ) {
        return Err(format!(
            "지원하지 않는 Iris endpoint method입니다: {method}"
        ));
    }
    let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
    let suffix = path
        .strip_prefix(prefix)
        .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'))
        .ok_or_else(|| "Iris endpoint URI가 설정된 prefix 밖에 있습니다".to_string())?;
    let lower_path = path.to_ascii_lowercase();
    if path.split('/').any(|segment| matches!(segment, "." | "..")) || lower_path.contains("%2e") {
        return Err("Iris endpoint URI에 허용되지 않는 경로가 있습니다".to_string());
    }
    let mut target = url::Url::parse(bridge_url).map_err(|error| error.to_string())?;
    let base_path = target.path().trim_end_matches('/').to_string();
    target.set_path(&format!(
        "{base_path}{}",
        if suffix.is_empty() { "/" } else { suffix }
    ));
    target.set_query((!query.is_empty()).then_some(query));
    iris_http_transaction(target.as_str(), token, method, content_type, body)
}

fn iris_http_transaction(
    url: &str,
    token: &str,
    method: &str,
    content_type: &str,
    payload: &[u8],
) -> Result<BridgeHttpResponse, String> {
    if [token, content_type]
        .into_iter()
        .any(|value| value.bytes().any(|byte| byte < b' ' || byte == 0x7f))
    {
        return Err("Iris 내부 브리지 HTTP 헤더 값이 올바르지 않습니다".to_string());
    }
    let parsed = url::Url::parse(url).map_err(|error| error.to_string())?;
    if parsed.scheme() != "http" {
        return Err("Iris 내부 브리지는 http URL만 지원합니다".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "Iris 내부 브리지 호스트가 없습니다".to_string())?;
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "Iris 내부 브리지 포트가 없습니다".to_string())?;
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .next()
        .ok_or_else(|| "Iris 내부 브리지 주소를 찾지 못했습니다".to_string())?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    let read_timeout = if parsed.path().ends_with("/vox/audio/stream") {
        Duration::from_secs(6 * 60 * 60)
    } else {
        Duration::from_secs(120)
    };
    stream
        .set_read_timeout(Some(read_timeout))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    let mut path = parsed.path().to_string();
    if path.is_empty() {
        path.push('/');
    }
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: {content_type}\r\nX-Noa-Hook-Token: {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .map_err(|error| error.to_string())?;
    stream
        .write_all(payload)
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut response = Vec::new();
    stream
        .take(MAX_IRIS_HTTP_RESPONSE_BYTES + 1)
        .read_to_end(&mut response)
        .map_err(|error| error.to_string())?;
    if response.len() as u64 > MAX_IRIS_HTTP_RESPONSE_BYTES {
        return Err("Noa endpoint 응답이 허용 크기를 초과했습니다".to_string());
    }
    parse_iris_http_response(&response)
}

fn parse_iris_http_response(response: &[u8]) -> Result<BridgeHttpResponse, String> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "잘못된 HTTP 응답 헤더".to_string())?;
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let mut lines = headers.lines();
    let status_line = lines.next().ok_or_else(|| "빈 HTTP 응답".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("잘못된 HTTP 상태행: {status_line}"))?;
    let fields = lines
        .filter_map(|line| line.split_once(':'))
        .collect::<Vec<_>>();
    let content_type = fields
        .iter()
        .find_map(|(name, value)| {
            name.eq_ignore_ascii_case("content-type")
                .then(|| value.trim().to_string())
        })
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let raw_body = &response[header_end + 4..];
    let body = if fields.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
    }) {
        decode_chunked_body(raw_body)?
    } else {
        raw_body.to_vec()
    };
    let body = String::from_utf8(body)
        .map_err(|_| "Noa endpoint 응답 본문은 UTF-8이어야 합니다".to_string())?;
    Ok(BridgeHttpResponse {
        status,
        content_type,
        body,
    })
}

fn decode_chunked_body(mut encoded: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    loop {
        let line_end = encoded
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| "잘못된 chunked HTTP 응답".to_string())?;
        let size_text = std::str::from_utf8(&encoded[..line_end])
            .map_err(|_| "잘못된 chunk 크기".to_string())?
            .split(';')
            .next()
            .unwrap_or_default()
            .trim();
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| "잘못된 chunk 크기".to_string())?;
        encoded = &encoded[line_end + 2..];
        if size == 0 {
            return Ok(decoded);
        }
        if encoded.len() < size + 2 || &encoded[size..size + 2] != b"\r\n" {
            return Err("완전하지 않은 chunked HTTP 응답".to_string());
        }
        decoded.extend_from_slice(&encoded[..size]);
        encoded = &encoded[size + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_response_status_type_and_body() {
        let parsed = parse_iris_http_response(
            b"HTTP/1.1 503 Unavailable\r\nContent-Type: application/json\r\n\r\n{\"ok\":false}",
        )
        .unwrap();
        assert_eq!(parsed.status, 503);
        assert_eq!(parsed.content_type, "application/json");
        assert_eq!(parsed.body, "{\"ok\":false}");
    }

    #[test]
    fn decodes_chunked_responses_with_extensions() {
        let parsed = parse_iris_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2;name=value\r\nhe\r\n3\r\nllo\r\n0\r\n\r\n",
        ).unwrap();
        assert_eq!(parsed.body, "hello");
    }

    #[test]
    fn rejects_truncated_chunks_and_invalid_utf8() {
        assert!(decode_chunked_body(b"5\r\nabc\r\n").is_err());
        assert!(parse_iris_http_response(b"HTTP/1.1 200 OK\r\n\r\n\xff").is_err());
    }

    #[test]
    fn rejects_paths_outside_endpoint_prefix_before_connecting() {
        for (path, expected) in [
            ("/noa-other/status", "prefix 밖"),
            ("/noa/../status", "허용되지 않는 경로"),
            ("/noa/%2e%2e/status", "허용되지 않는 경로"),
        ] {
            let error = forward_iris_endpoint_http(
                "http://127.0.0.1:1/api",
                "/noa",
                "token",
                "GET",
                path,
                "application/json",
                b"",
            )
            .unwrap_err();
            assert!(error.contains(expected), "{path}: {error}");
        }
    }
}
