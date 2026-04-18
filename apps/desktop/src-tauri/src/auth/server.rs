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

/// Inclusive lower bound of the OAuth callback port range.
///
/// Every value in `CALLBACK_PORT_MIN..=CALLBACK_PORT_MAX` must be registered
/// as an allowed redirect URI in the GitHub / Google OAuth application, since
/// providers do not honor wildcards. Keep the range small.
pub const CALLBACK_PORT_MIN: u16 = 5678;
/// Inclusive upper bound of the OAuth callback port range.
pub const CALLBACK_PORT_MAX: u16 = 5685;
/// Fixed path for the OAuth callback endpoint.
pub const CALLBACK_PATH: &str = "/auth/github/callback";

/// Starts a temporary localhost HTTP server that waits for one OAuth callback.
///
/// Tries to bind each port in `CALLBACK_PORT_MIN..=CALLBACK_PORT_MAX` in order
/// and returns the first one that succeeds; fails only when every port in the
/// range is already in use. Returns the chosen port alongside a future that
/// resolves to the callback result (#293).
pub async fn start_callback_server() -> Result<(u16, tokio::task::JoinHandle<Result<CallbackResult, String>>), String> {
    let (listener, port) = bind_in_range(CALLBACK_PORT_MIN, CALLBACK_PORT_MAX).await?;

    let handle = tokio::spawn(async move {
        let result = timeout(CALLBACK_TIMEOUT, accept_callback(&listener)).await;
        match result {
            Ok(inner) => inner,
            Err(_) => Err("OAuth callback timed out after 120s".to_string()),
        }
    });

    Ok((port, handle))
}

async fn bind_in_range(min: u16, max: u16) -> Result<(TcpListener, u16), String> {
    let mut last_err: Option<String> = None;
    for port in min..=max {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return Ok((listener, port)),
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    Err(format!(
        "failed to bind OAuth callback server: every port in {min}..={max} is in use \
         (close a duplicate Nebula instance or conflicting process); last error: {}",
        last_err.unwrap_or_else(|| "none".to_string())
    ))
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
        let req = "GET /auth/github/callback?code=abc123&state=xyz HTTP/1.1\r\nHost: localhost\r\n\r\n";
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

    /// Regression for #293: when the first port in the range is already in
    /// use, `bind_in_range` must fall through to the next free port instead
    /// of returning `AddrInUse`.
    #[tokio::test]
    async fn bind_in_range_falls_through_to_next_free_port() {
        // Hold the first port in the range so the first bind attempt fails.
        let _blocker = TcpListener::bind(("127.0.0.1", CALLBACK_PORT_MIN))
            .await
            .expect("first port should be bindable in a fresh test env");

        let (_listener, port) = bind_in_range(CALLBACK_PORT_MIN, CALLBACK_PORT_MAX)
            .await
            .expect("second port in range should be free");
        assert!(
            port > CALLBACK_PORT_MIN && port <= CALLBACK_PORT_MAX,
            "expected port in range ({}, {}], got {}",
            CALLBACK_PORT_MIN,
            CALLBACK_PORT_MAX,
            port,
        );
    }

    /// Regression for #293: when every port in the range is occupied, the
    /// helper must return an actionable error, not a misleading "port 5678
    /// in use" message for a range bind.
    #[tokio::test]
    async fn bind_in_range_reports_when_fully_saturated() {
        // Grab a single narrow two-port range and occupy both ports.
        let a = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("ephemeral bind works");
        let b = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("ephemeral bind works");
        let port_a = a.local_addr().unwrap().port();
        let port_b = b.local_addr().unwrap().port();
        let (lo, hi) = if port_a <= port_b {
            (port_a, port_b)
        } else {
            (port_b, port_a)
        };

        let err = bind_in_range(lo, hi)
            .await
            .expect_err("range fully occupied must error");
        assert!(
            err.contains("every port"),
            "error must explain that the whole range was tried, got: {err}",
        );
    }
}
