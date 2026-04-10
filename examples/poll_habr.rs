//! `PollAction` DX trait demo — polls Habr's RSS feed for fresh articles.
//!
//! Run:
//!
//! ```bash
//! cargo run -p nebula-examples --bin poll_habr
//! ```
//!
//! The action polls https://habr.com/ru/rss/articles/ every 500ms. Cursor is
//! the newest `pub_date` seen in the previous cycle (parsed as RFC 2822 via
//! chrono). First cycle emits every item in the feed; subsequent cycles only
//! emit items with strictly newer `pub_date`.
//!
//! `main` drives the run through `TestRuntime::run_poll` with a 2.5-second
//! window — the runtime spawns `start()`, sleeps, cancels, and returns
//! everything the spy emitter captured.

use std::time::Duration;

use nebula_sdk::prelude::*;

// ── Action definition ──────────────────────────────────────────────────────

const HABR_RSS_URL: &str = "https://habr.com/ru/rss/articles/";
const HABR_POLL_INTERVAL: Duration = Duration::from_millis(500);

struct HabrRssPollAction {
    meta: ActionMetadata,
}

impl HabrRssPollAction {
    fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                action_key!("habr.rss"),
                "Habr RSS Poll",
                "Poll https://habr.com/ru/rss/articles/ via PollAction DX trait",
            ),
        }
    }
}

impl ActionDependencies for HabrRssPollAction {}
impl Action for HabrRssPollAction {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

#[derive(Serialize)]
struct HabrArticle {
    title: String,
    link: String,
    pub_date: String,
    author: String,
}

fn build_habr_article(item: &rss::Item) -> HabrArticle {
    HabrArticle {
        title: item.title().unwrap_or("<no title>").to_owned(),
        link: item.link().unwrap_or("").to_owned(),
        pub_date: item.pub_date().unwrap_or("").to_owned(),
        author: item
            .author()
            .or_else(|| {
                item.dublin_core_ext()
                    .and_then(|dc| dc.creators().first().map(String::as_str))
            })
            .unwrap_or("<unknown>")
            .to_owned(),
    }
}

fn parse_rfc2822(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc2822(raw)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

impl PollAction for HabrRssPollAction {
    type Cursor = Option<chrono::DateTime<chrono::Utc>>;
    type Event = HabrArticle;

    fn poll_interval(&self) -> Duration {
        HABR_POLL_INTERVAL
    }

    async fn poll(
        &self,
        cursor: &mut Self::Cursor,
        _ctx: &TriggerContext,
    ) -> Result<Vec<Self::Event>, ActionError> {
        let response = reqwest::get(HABR_RSS_URL).await.map_err(|e| {
            if e.is_timeout() || e.is_connect() {
                ActionError::retryable(format!("habr RSS fetch failed: {e}"))
            } else {
                ActionError::fatal(format!("habr RSS fetch failed: {e}"))
            }
        })?;

        if !response.status().is_success() {
            return Err(ActionError::retryable(format!(
                "habr RSS returned HTTP {}",
                response.status()
            )));
        }

        let body = response
            .bytes()
            .await
            .map_err(|e| ActionError::retryable(format!("habr RSS body read failed: {e}")))?;

        let channel = rss::Channel::read_from(&body[..])
            .map_err(|e| ActionError::fatal(format!("habr RSS parse failed: {e}")))?;

        let items = channel.items();
        let mut new_events: Vec<HabrArticle> = Vec::new();
        let mut newest_seen: Option<chrono::DateTime<chrono::Utc>> = *cursor;

        for item in items {
            let Some(pub_date) = parse_rfc2822(item.pub_date().unwrap_or("")) else {
                continue;
            };
            if let Some(c) = *cursor
                && pub_date <= c
            {
                break;
            }
            new_events.push(build_habr_article(item));
            newest_seen = Some(newest_seen.map_or(pub_date, |n| n.max(pub_date)));
        }

        *cursor = newest_seen;
        Ok(new_events)
    }
}

// ── Runner ─────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let window = Duration::from_millis(2500);
    println!("▶ poll_habr — running the trigger for {}ms", window.as_millis());
    println!();

    let ctx = TestContextBuilder::new();
    let report = TestRuntime::new(ctx)
        .with_trigger_window(window)
        .run_poll(HabrRssPollAction::new())
        .await?;

    println!("kind:       {}", report.kind);
    println!("iterations: {}", report.iterations);
    println!("duration:   {:?}", report.duration);
    if let Some(note) = &report.note {
        println!("note:       {note}");
    }
    println!();
    println!("emitted {} execution(s) over the window", report.emitted.len());
    println!();

    for (i, evt) in report.emitted.iter().enumerate() {
        let title = evt.get("title").and_then(|v| v.as_str()).unwrap_or("?");
        let pub_date = evt.get("pub_date").and_then(|v| v.as_str()).unwrap_or("?");
        let author = evt.get("author").and_then(|v| v.as_str()).unwrap_or("?");
        println!("  [{:>3}] {pub_date}  {author}", i + 1);
        println!("        {title}");
    }

    Ok(())
}
