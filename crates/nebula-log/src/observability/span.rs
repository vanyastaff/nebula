//! Span-like resource merging across nested contexts
//!
//! Like `tracing` spans, observability contexts can be nested.
//! This module provides utilities for merging resources from parent spans.

use super::context::{ExecutionContext, NodeContext};
use super::resources::LoggerResource;

/// Get merged LoggerResource from all active contexts (span-like)
///
/// Merges resources in this order (lower overrides higher):
/// 1. Execution context (if active)
/// 2. Node context (if active)
///
/// This mimics `tracing` span behavior where child spans inherit parent attributes.
pub fn get_current_logger_resource() -> Option<LoggerResource> {
    let mut base = LoggerResource::default();
    let mut found_any = false;

    // 1. Merge from Execution context (lower priority)
    if let Some(exec) = ExecutionContext::current()
        && let Some(logger) = exec.resources.get::<LoggerResource>()
    {
        base = merge_logger_resources(base, (*logger).clone());
        found_any = true;
    }

    // 2. Merge from Node context (highest priority)
    if let Some(node) = NodeContext::current()
        && let Some(logger) = node.get_resource::<LoggerResource>()
    {
        base = merge_logger_resources(base, (*logger).clone());
        found_any = true;
    }

    if found_any { Some(base) } else { None }
}

/// Merge two LoggerResources (second overrides first)
///
/// Rules:
/// - Tags: accumulated (both kept)
/// - Sentry DSN: replaced if present in override
/// - Webhook: replaced if present in override
/// - Log level: replaced if different from default
/// - Notifications: merged
/// - Sampling: override if enabled
fn merge_logger_resources(base: LoggerResource, override_with: LoggerResource) -> LoggerResource {
    let mut result = base;

    // Merge Sentry DSN (override if present)
    if override_with.sentry_dsn.is_some() {
        result.sentry_dsn = override_with.sentry_dsn;
    }

    // Merge webhook URL (override if present)
    if override_with.webhook_url.is_some() {
        result.webhook_url = override_with.webhook_url;
    }

    // Merge log level (override if not default)
    if !matches!(override_with.log_level, super::resources::LogLevel::Info) {
        result.log_level = override_with.log_level;
    }

    // Accumulate tags (both base and override)
    result.tags.extend(override_with.tags);

    // Merge notification preferences
    if override_with.notification_prefs.email_enabled {
        result.notification_prefs.email_enabled = true;
        if !override_with.notification_prefs.email_addresses.is_empty() {
            result.notification_prefs.email_addresses =
                override_with.notification_prefs.email_addresses;
        }
    }
    if override_with.notification_prefs.webhook_enabled {
        result.notification_prefs.webhook_enabled = true;
    }
    if override_with.notification_prefs.min_severity
        != super::resources::NotificationSeverity::Error
    {
        result.notification_prefs.min_severity = override_with.notification_prefs.min_severity;
    }
    if override_with.notification_prefs.rate_limit_per_hour != 10 {
        result.notification_prefs.rate_limit_per_hour =
            override_with.notification_prefs.rate_limit_per_hour;
    }

    // Merge sampling
    if override_with.sampling_enabled {
        result.sampling_enabled = true;
        result.sampling_rate = override_with.sampling_rate;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_span_like_merging() {
        // Execution context: Sentry + tag
        let exec = ExecutionContext::new("exec-1", "wf-1", "tenant-1").with_resource(
            LoggerResource::new()
                .with_sentry_dsn("https://exec@sentry.io/project")
                .with_tag("execution_id", "exec-1"),
        );

        let _exec_guard = exec.enter();

        // Node context: webhook + tag
        let node = NodeContext::new("node-1", "action-1").with_resource(
            LoggerResource::new()
                .with_webhook("https://hooks.slack.com/...")
                .with_tag("node_id", "node-1"),
        );

        let _node_guard = node.enter();

        // Get merged resource
        let merged = get_current_logger_resource().unwrap();

        // Should have Sentry from Execution
        assert_eq!(merged.sentry_dsn(), Some("https://exec@sentry.io/project"));

        // Should have webhook from Node
        assert_eq!(merged.webhook_url(), Some("https://hooks.slack.com/..."));

        // Should have both tags (accumulated)
        assert_eq!(merged.tags.len(), 2);
        let tag_keys: Vec<_> = merged.tags.iter().map(|(k, _)| k.as_str()).collect();
        assert!(tag_keys.contains(&"execution_id"));
        assert!(tag_keys.contains(&"node_id"));
    }

    #[test]
    fn test_override_sentry() {
        // Execution: default Sentry
        let exec = ExecutionContext::new("exec-1", "wf-1", "tenant-1")
            .with_resource(LoggerResource::new().with_sentry_dsn("https://exec@sentry.io/project"));

        let _exec_guard = exec.enter();

        // Node: override with different Sentry
        let node = NodeContext::new("node-1", "action-1")
            .with_resource(LoggerResource::new().with_sentry_dsn("https://node@sentry.io/other"));

        let _node_guard = node.enter();

        let merged = get_current_logger_resource().unwrap();

        // Should use Node's Sentry (overrides Execution)
        assert_eq!(merged.sentry_dsn(), Some("https://node@sentry.io/other"));
    }

    #[test]
    fn test_no_contexts() {
        // No contexts active
        let merged = get_current_logger_resource();
        assert!(merged.is_none());
    }

    #[test]
    fn test_single_context() {
        let exec = ExecutionContext::new("exec-1", "wf-1", "tenant-1").with_resource(
            LoggerResource::new()
                .with_sentry_dsn("https://test@sentry.io/project")
                .with_tag("test", "value"),
        );

        let _guard = exec.enter();

        let merged = get_current_logger_resource().unwrap();
        assert_eq!(merged.sentry_dsn(), Some("https://test@sentry.io/project"));
        assert_eq!(merged.tags.len(), 1);
    }
}
