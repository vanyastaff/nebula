use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

/// Result of a successful OAuth callback capture.
#[derive(Debug)]
pub struct CallbackResult {
    pub code: String,
    pub state: String,
}

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(120);

const SUCCESS_HTML: &str = r#"<!DOCTYPE html>
<html><head><title>Nebula</title></head>
<body style="font-family:sans-serif;text-align:center;padding:3rem">
<h2>Authentication successful</h2>
<p>You can close this tab and return to Nebula.</p>
</body></html>"#;

const ERROR_HTML: &str = r#"<!DOCTYPE html>
<html><head><title>Nebula</title></head>
<body style="font-family:sans-serif;text-align:center;padding:3rem">
<h2>Authentication failed</h2>
<p>Missing required parameters. Please try again.</p>
</body></html>"#;

/// Starts a temporary localhost HTTP server that waits for one OAuth callback.
///
/// Binds to `127.0.0.1:0` (OS-assigned port), accepts a single connection,
/// parses the `code` and `state` query parameters, and shuts down.
///
/// Returns the port and a future that resolves to the callback result.
pub async fn start_callback_server() -> Result<(u16, tokio::task::JoinHandle<Result<CallbackResult, String>>), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("failed to bind callback server: {e}"))?;

    let port = listener
        .local_addr()
        .map_err(|e| format!("failed to get local addr: {e}"))?
        .port();

    let handle = tokio::spawn(async move {
        let result = timeout(CALLBACK_TIMEOUT, accept_callback(&listener)).await;
        match result {
            Ok(inner) => inner,
            Err(_) => Err("OAuth callback timed out after 120s".to_string()),
        }
    });

    Ok((port, handle))
}

async fn accept_callback(listener: &TcpListener) -> Result<CallbackResult, String> {
    let (mut stream, _addr) = listener
        .accept()
        .await
        .map_err(|e| format!("accept failed: {e}"))?;

    let mut buf = vec![0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("read failed: {e}"))?;

    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse GET /callback?code=...&state=... HTTP/1.1
    let result = parse_callback_params(&request);

    let (status_line, body) = match &result {
        Ok(_) => ("HTTP/1.1 200 OK", SUCCESS_HTML),
        Err(_) => ("HTTP/1.1 400 Bad Request", ERROR_HTML),
    };

    let response = format!(
        "{status_line}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;

    result
}

fn parse_callback_params(request: &str) -> Result<CallbackResult, String> {
    // Extract the request path from "GET /callback?... HTTP/1.1"
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or("invalid HTTP request")?;

    let query = path
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("");

    let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let code = params
        .iter()
        .find(|(k, _)| k == "code")
        .map(|(_, v)| v.clone())
        .ok_or("missing 'code' parameter")?;

    let state = params
        .iter()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.clone())
        .ok_or("missing 'state' parameter")?;

    Ok(CallbackResult { code, state })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_callback() {
        let req = "GET /callback?code=abc123&state=xyz HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let result = parse_callback_params(req).unwrap();
        assert_eq!(result.code, "abc123");
        assert_eq!(result.state, "xyz");
    }

    #[test]
    fn parse_missing_code() {
        let req = "GET /callback?state=xyz HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert!(parse_callback_params(req).is_err());
    }

    #[test]
    fn parse_missing_state() {
        let req = "GET /callback?code=abc HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert!(parse_callback_params(req).is_err());
    }
}
