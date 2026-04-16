//! Error recovery suggestions — deterministic, schema-aware hints.
//!
//! Analyzes node errors after execution and suggests fixes.
//! No AI, no guessing — finite set of known failure patterns.

use nebula_engine::ExecutionResult;

/// A recovery suggestion for a failed node.
pub struct Suggestion {
    /// Node ID that failed.
    pub node_key: String,
    /// What went wrong (short).
    pub problem: String,
    /// What to do about it.
    pub fix: String,
    /// Optional YAML snippet to add.
    pub yaml: Option<String>,
}

/// Analyze execution result and generate recovery suggestions.
pub fn suggest(result: &ExecutionResult) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();

    for (node_key, error) in &result.node_errors {
        let error_lower = error.to_lowercase();
        let nid = node_key.to_string();

        // HTTP 429 — Too Many Requests
        if error_lower.contains("429") || error_lower.contains("rate limit") {
            suggestions.push(Suggestion {
                node_key: nid.clone(),
                problem: "HTTP 429 Too Many Requests".into(),
                fix: "Add a rate_limit to this node to throttle requests.".into(),
                yaml: Some("    rate_limit:\n      max_requests: 30\n      window_secs: 60".into()),
            });
        }

        // Timeout
        if error_lower.contains("timeout") || error_lower.contains("timed out") {
            suggestions.push(Suggestion {
                node_key: nid.clone(),
                problem: "Request timed out".into(),
                fix: "Increase the node timeout or the --timeout flag.".into(),
                yaml: Some("    timeout: 60s".into()),
            });
        }

        // Connection refused / failed
        if error_lower.contains("connection refused")
            || error_lower.contains("connection failed")
            || error_lower.contains("connect error")
        {
            suggestions.push(Suggestion {
                node_key: nid.clone(),
                problem: "Connection failed".into(),
                fix: "Check that the target service is running and the URL is correct.".into(),
                yaml: None,
            });
        }

        // DNS resolution
        if error_lower.contains("dns") || error_lower.contains("resolve") {
            suggestions.push(Suggestion {
                node_key: nid.clone(),
                problem: "DNS resolution failed".into(),
                fix: "Check the hostname in the URL. Is it spelled correctly?".into(),
                yaml: None,
            });
        }

        // Action not found
        if error_lower.contains("action not found") {
            // Extract the action key from the error message.
            let key = error
                .split("action not found:")
                .nth(1)
                .map(|s| s.trim())
                .unwrap_or("unknown");

            suggestions.push(Suggestion {
                node_key: nid.clone(),
                problem: format!("Action \"{key}\" is not registered"),
                fix: format!(
                    "Check the action_key in your workflow.\n\
                     Run `nebula actions list` to see available built-in actions.\n\
                     If this is a community plugin, install it first: \
                     `nebula plugin install {key}`"
                ),
                yaml: None,
            });
        }

        // Validation error (bad input)
        if error_lower.contains("validation")
            || error_lower.contains("missing required field")
            || error_lower.contains("invalid input")
        {
            suggestions.push(Suggestion {
                node_key: nid.clone(),
                problem: "Input validation failed".into(),
                fix: "Check the parameters for this node. \
                      Run `nebula actions info <key>` to see expected parameters."
                    .into(),
                yaml: None,
            });
        }

        // HTTP 401/403
        if error_lower.contains("401") || error_lower.contains("unauthorized") {
            suggestions.push(Suggestion {
                node_key: nid.clone(),
                problem: "Authentication failed (HTTP 401)".into(),
                fix: "Check your credentials. Is the API key or token expired?".into(),
                yaml: None,
            });
        }
        if error_lower.contains("403") || error_lower.contains("forbidden") {
            suggestions.push(Suggestion {
                node_key: nid.clone(),
                problem: "Access denied (HTTP 403)".into(),
                fix: "Check permissions for your API key. Does it have the required scopes?".into(),
                yaml: None,
            });
        }

        // HTTP 500+
        if error_lower.contains("500")
            || error_lower.contains("502")
            || error_lower.contains("503")
            || error_lower.contains("internal server error")
        {
            suggestions.push(Suggestion {
                node_key: nid.clone(),
                problem: "Server error (5xx)".into(),
                fix: "The remote service returned a server error. \
                      This is usually temporary — add a retry policy."
                    .into(),
                yaml: Some(
                    "    retry_policy:\n      max_attempts: 3\n      backoff: exponential".into(),
                ),
            });
        }

        // Plugin process crashed
        if error_lower.contains("plugin exited with") {
            suggestions.push(Suggestion {
                node_key: nid.clone(),
                problem: "Plugin process crashed".into(),
                fix: "The community plugin binary exited with a non-zero status.\n\
                      Run the plugin binary directly to debug:\n\
                      echo '{\"action_key\":\"...\",\"input\":{}}' | ./plugins/nebula-plugin-<name>"
                    .into(),
                yaml: None,
            });
        }
    }

    suggestions
}

/// Print suggestions to stderr.
pub fn print_suggestions(suggestions: &[Suggestion]) {
    if suggestions.is_empty() {
        return;
    }

    eprintln!();
    eprintln!("💡 Suggestions:");
    eprintln!();

    for (i, s) in suggestions.iter().enumerate() {
        eprintln!("  {}. Node {}", i + 1, s.node_key);
        eprintln!("     Problem: {}", s.problem);
        eprintln!("     Fix:     {}", s.fix);
        if let Some(yaml) = &s.yaml {
            eprintln!("     Add to workflow YAML:");
            for line in yaml.lines() {
                eprintln!("       {line}");
            }
        }
        eprintln!();
    }
}
