use chrono::{Days, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;

use crate::constants::{
    AppResult, DEFAULT_END_DATE, DEFAULT_START_DATE, ZULIP_CHUNK_SUMMARIES_PATH,
    ZULIP_MESSAGE_CHAR_LIMIT, ZULIP_OUTPUT_PATH, ZULIP_PAGE_SIZE, ZULIP_SUMMARY_CHUNK_SIZE,
    ZULIP_SUMMARY_PATH, ZULIP_TOPIC_INPUT_PATH, ZULIP_TOPIC_SUMMARY_PATH,
};
use crate::openai::create_chat_completion;
use crate::util::{
    compact_whitespace, display_recipient_to_stream_name, parse_utc_range, pick_date,
    timestamp_to_rfc3339, truncate_chars,
};

#[derive(Debug, Deserialize)]
struct ZulipMessagesResponse {
    messages: Vec<ZulipMessage>,
    #[serde(rename = "found_oldest")]
    found_oldest: bool,
}

#[derive(Debug, Deserialize)]
struct ZulipMessage {
    id: u64,
    timestamp: i64,
    #[serde(default, rename = "type")]
    message_type: String,
    #[serde(default)]
    display_recipient: serde_json::Value,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    sender_full_name: String,
    #[serde(default)]
    sender_email: String,
    #[serde(default)]
    content: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ZulipOutputMessage {
    id: u64,
    timestamp: i64,
    datetime: String,
    stream: String,
    topic: String,
    sender_full_name: String,
    sender_email: String,
    content: String,
}

pub async fn gather_messages(
    start_date_override: Option<&str>,
    end_date_override: Option<&str>,
    duration_days_override: Option<u64>,
) -> AppResult<()> {
    let base_url = env::var("ZULIP_BASE_URL")
        .map_err(|_| "ZULIP_BASE_URL is required. Example: https://your-org.zulipchat.com")?;
    let email =
        env::var("ZULIP_EMAIL").map_err(|_| "ZULIP_EMAIL is required for Zulip authentication.")?;
    let api_key = env::var("ZULIP_API_KEY")
        .map_err(|_| "ZULIP_API_KEY is required for Zulip authentication.")?;

    let (start_date, end_date_label, start_ts, end_exclusive_ts, using_duration) =
        if let Some(duration_days) = duration_days_override {
            let now = Utc::now();
            let today = now.date_naive();
            let start_day = today
                .checked_sub_days(Days::new(duration_days))
                .ok_or("Could not compute start date from --duration-days.")?;
            let start_dt = start_day
                .and_hms_opt(0, 0, 0)
                .ok_or("Could not build start datetime at midnight UTC.")?;
            (
                start_day.format("%Y-%m-%d").to_string(),
                format!("now ({})", now.to_rfc3339()),
                start_dt.and_utc().timestamp(),
                now.timestamp(),
                true,
            )
        } else {
            let start_date = pick_date(
                start_date_override,
                &["ZULIP_START_DATE", "START_DATE", "start_date"],
                DEFAULT_START_DATE,
            );
            let end_date = pick_date(
                end_date_override,
                &["ZULIP_END_DATE", "END_DATE", "end_date"],
                DEFAULT_END_DATE,
            );
            let (start_ts, end_exclusive_ts) = parse_utc_range(&start_date, &end_date)?;
            (start_date, end_date, start_ts, end_exclusive_ts, false)
        };

    if using_duration {
        println!(
            "Fetching Zulip stream messages from {} 00:00:00 UTC to {} across all channels/topics",
            start_date, end_date_label
        );
    } else {
        println!(
            "Fetching Zulip stream messages from {} to {} (UTC) across all channels/topics",
            start_date, end_date_label
        );
    }

    let normalized_base = base_url.trim_end_matches('/');
    let messages_url = if normalized_base.ends_with("/api/v1") {
        format!("{normalized_base}/messages")
    } else {
        format!("{normalized_base}/api/v1/messages")
    };
    let mut anchor = "newest".to_string();
    let mut collected: Vec<ZulipOutputMessage> = Vec::new();
    let client = Client::new();

    loop {
        let page_size = ZULIP_PAGE_SIZE.to_string();
        let response = client
            .get(&messages_url)
            .basic_auth(&email, Some(&api_key))
            .query(&[
                ("anchor", anchor.as_str()),
                ("num_before", page_size.as_str()),
                ("num_after", "0"),
                ("apply_markdown", "false"),
            ])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(format!("Zulip API error {status}: {error_body}").into());
        }

        let page = response.json::<ZulipMessagesResponse>().await?;
        if page.messages.is_empty() {
            break;
        }

        let oldest_id = page.messages.first().map(|message| message.id).unwrap_or(0);
        let oldest_ts = page
            .messages
            .first()
            .map(|message| message.timestamp)
            .unwrap_or(i64::MAX);

        for message in page.messages {
            if message.message_type != "stream" {
                continue;
            }

            if message.timestamp < start_ts || message.timestamp >= end_exclusive_ts {
                continue;
            }

            let topic = if !message.topic.is_empty() {
                message.topic
            } else {
                message.subject
            };

            collected.push(ZulipOutputMessage {
                id: message.id,
                timestamp: message.timestamp,
                datetime: timestamp_to_rfc3339(message.timestamp),
                stream: display_recipient_to_stream_name(&message.display_recipient),
                topic,
                sender_full_name: message.sender_full_name,
                sender_email: message.sender_email,
                content: message.content,
            });
        }

        if page.found_oldest || oldest_id == 0 || oldest_ts < start_ts {
            break;
        }

        anchor = oldest_id.saturating_sub(1).to_string();
    }

    collected.sort_by_key(|message| (message.timestamp, message.id));

    fs::create_dir_all("./inputs")?;
    let json = serde_json::to_string_pretty(&collected)?;
    fs::write(ZULIP_OUTPUT_PATH, format!("{json}\n"))?;

    println!(
        "Zulip gather done. Wrote {} messages to {}",
        collected.len(),
        ZULIP_OUTPUT_PATH
    );
    Ok(())
}

fn read_zulip_messages() -> AppResult<Vec<ZulipOutputMessage>> {
    let raw = fs::read_to_string(ZULIP_OUTPUT_PATH)?;
    let mut items = serde_json::from_str::<Vec<ZulipOutputMessage>>(&raw)?;
    items.sort_by_key(|message| (message.timestamp, message.id));
    Ok(items)
}

fn build_zulip_chunk_prompt(messages: &[ZulipOutputMessage]) -> String {
    let mut content = String::new();

    for message in messages {
        let msg_id = message.id.to_string();
        let stream = message.stream.trim();
        let topic = message.topic.trim();
        let sender = message.sender_full_name.trim();
        let body = truncate_chars(
            &compact_whitespace(message.content.as_str()),
            ZULIP_MESSAGE_CHAR_LIMIT,
        );

        content.push_str("- MSG_ID=");
        content.push_str(msg_id.as_str());
        content.push('\n');
        content.push_str("  DATETIME=");
        content.push_str(message.datetime.as_str());
        content.push('\n');
        content.push_str("  STREAM=");
        content.push_str(stream);
        content.push('\n');
        content.push_str("  TOPIC=");
        content.push_str(topic);
        content.push('\n');
        content.push_str("  SENDER=");
        content.push_str(sender);
        content.push('\n');
        content.push_str("  CONTENT=");
        content.push_str(body.as_str());
        content.push_str("\n\n");
    }

    content
}

fn compare_topic_key(a: &ZulipOutputMessage, b: &ZulipOutputMessage) -> Ordering {
    a.stream
        .cmp(&b.stream)
        .then_with(|| a.topic.cmp(&b.topic))
        .then_with(|| a.timestamp.cmp(&b.timestamp))
        .then_with(|| a.id.cmp(&b.id))
}

fn push_topic_group_chunks(
    chunks: &mut Vec<Vec<ZulipOutputMessage>>,
    group: Vec<ZulipOutputMessage>,
    chunk_size: usize,
) {
    for slice in group.chunks(chunk_size) {
        chunks.push(slice.to_vec());
    }
}

fn build_zulip_topic_chunks(
    mut messages: Vec<ZulipOutputMessage>,
    chunk_size: usize,
) -> Vec<Vec<ZulipOutputMessage>> {
    if messages.is_empty() {
        return Vec::new();
    }

    messages.sort_by(compare_topic_key);

    let mut chunks: Vec<Vec<ZulipOutputMessage>> = Vec::new();
    let mut group: Vec<ZulipOutputMessage> = Vec::new();
    let mut current_key: Option<(String, String)> = None;

    for message in messages {
        let key = (message.stream.clone(), message.topic.clone());
        if current_key.as_ref() != Some(&key) {
            if !group.is_empty() {
                push_topic_group_chunks(&mut chunks, std::mem::take(&mut group), chunk_size);
            }
            current_key = Some(key);
        }
        group.push(message);
    }

    if !group.is_empty() {
        push_topic_group_chunks(&mut chunks, group, chunk_size);
    }

    chunks
}

fn chunk_scope(messages: &[ZulipOutputMessage]) -> String {
    if let Some(first) = messages.first() {
        format!("stream={} topic={}", first.stream, first.topic)
    } else {
        "stream=unknown topic=unknown".to_string()
    }
}

async fn ask_chatgpt_to_summarize_zulip_chunk(
    client: &Client,
    api_key: &str,
    model: &str,
    scope: &str,
    messages: &[ZulipOutputMessage],
) -> AppResult<String> {
    let chunk_content = build_zulip_chunk_prompt(messages);
    let prompt = [
        "You are analyzing engineering discussion from Zulip.",
        "The input is pre-grouped to maximize messages from the same channel/topic.",
        &format!("Chunk scope: {scope}"),
        "Write a detailed yet concise technical summary for stakeholders.",
        "Prioritize reasoning, actions, plans, and impacts.",
        "Hard rules:",
        "1) Use only the provided messages.",
        "2) Explain why decisions were made when evidence exists.",
        "3) Call out actions completed and planned next actions.",
        "4) Highlight impact, risks, and dependencies.",
        "5) If uncertain/conflicting, state that explicitly.",
        "6) Keep each section as numbered lines with no sub-bullets.",
        "7) Explicitly detect action items where the message content contains the exact tag `@**Qianchen**`.",
        "8) For `QIANCHEN_TAGGED_ACTION_ITEMS`, include action, owner/context, and supporting `MSG_ID=<...>`.",
        "Output format:",
        "CHUNK_OVERVIEW:",
        "1. ...",
        "TECHNICAL_DECISIONS_AND_REASONING:",
        "1. ...",
        "ACTIONS_COMPLETED:",
        "1. ...",
        "PLANNED_ACTIONS:",
        "1. ...",
        "QIANCHEN_TAGGED_ACTION_ITEMS:",
        "1. ... MSG_ID=<...>",
        "IMPACTS_AND_RISKS:",
        "1. ...",
        "OPEN_QUESTIONS:",
        "1. ...",
        "",
        "Messages:",
        &chunk_content,
    ]
    .join("\n");

    create_chat_completion(client, api_key, model, prompt).await
}

async fn ask_chatgpt_to_merge_zulip_summaries(
    client: &Client,
    api_key: &str,
    model: &str,
    chunk_summaries: &str,
) -> AppResult<String> {
    let prompt = [
        "You are preparing a final report from multiple Zulip chunk summaries.",
        "Goal: detailed yet concise explanation that helps users understand technical reasoning.",
        "Requirements:",
        "1) Merge overlaps and remove repetition.",
        "2) Preserve concrete technical details and rationale.",
        "3) Clearly separate completed actions vs planned actions.",
        "4) Explain impact on users/systems and likely outcomes.",
        "5) Include risks, blockers, and open questions.",
        "6) Use numbered lines only under each section, no sub-bullets.",
        "7) Keep claims grounded in the chunk summaries.",
        "8) Include a dedicated section for action items where `@**Qianchen**` was explicitly tagged.",
        "Output format:",
        "# Zulip Discussion Summary",
        "## Executive Summary",
        "1. ...",
        "## Key Technical Decisions And Reasoning",
        "1. ...",
        "## Actions Completed",
        "1. ...",
        "## Planned Actions",
        "1. ...",
        "## Action Items Tagged @**Qianchen**",
        "1. ...",
        "## Expected Impact",
        "1. ...",
        "## Risks And Open Questions",
        "1. ...",
        "",
        "Chunk summaries:",
        chunk_summaries,
    ]
    .join("\n");

    create_chat_completion(client, api_key, model, prompt).await
}

pub async fn summarize_discussion(client: &Client, api_key: &str, model: &str) -> AppResult<()> {
    println!("Starting Zulip discussion summary...");
    let messages = read_zulip_messages()?;
    if messages.is_empty() {
        eprintln!("No messages found in {ZULIP_OUTPUT_PATH}. Run the zulip command first.");
        return Ok(());
    }

    let chunks = build_zulip_topic_chunks(messages, ZULIP_SUMMARY_CHUNK_SIZE);
    let total_messages: usize = chunks.iter().map(|chunk| chunk.len()).sum();
    println!(
        "Summarizing {} messages in {} chunks grouped by stream/topic...",
        total_messages,
        chunks.len()
    );

    fs::create_dir_all("./outputs")?;
    fs::write(ZULIP_CHUNK_SUMMARIES_PATH, "")?;

    let mut chunk_output = OpenOptions::new()
        .append(true)
        .open(ZULIP_CHUNK_SUMMARIES_PATH)?;
    let mut partial_summaries: Vec<String> = Vec::new();

    for (index, chunk) in chunks.iter().enumerate() {
        let scope = chunk_scope(chunk);
        println!(
            "Summarizing Zulip chunk {}/{} ({})...",
            index + 1,
            chunks.len(),
            scope
        );
        match ask_chatgpt_to_summarize_zulip_chunk(client, api_key, model, &scope, chunk).await {
            Ok(result) => {
                let trimmed = result.trim_end();
                writeln!(chunk_output, "CHUNK_{} [{}]:", index + 1, scope)?;
                writeln!(chunk_output, "{trimmed}\n")?;
                partial_summaries.push(format!("CHUNK_{} [{}]:\n{trimmed}", index + 1, scope));
            }
            Err(err) => {
                eprintln!("Chunk {} failed: {err}", index + 1);
            }
        }
    }

    if partial_summaries.is_empty() {
        return Err("All Zulip summary chunks failed.".into());
    }

    let merged_chunks = partial_summaries.join("\n\n");
    let final_summary =
        ask_chatgpt_to_merge_zulip_summaries(client, api_key, model, &merged_chunks).await?;

    fs::write(
        ZULIP_SUMMARY_PATH,
        format!("{}\n", final_summary.trim_end()),
    )?;
    println!(
        "Zulip summary done. Wrote {} and {}",
        ZULIP_CHUNK_SUMMARIES_PATH, ZULIP_SUMMARY_PATH
    );
    Ok(())
}

pub async fn run_report(
    client: &Client,
    api_key: &str,
    model: &str,
    start_date_override: Option<&str>,
    end_date_override: Option<&str>,
    duration_days_override: Option<u64>,
) -> AppResult<()> {
    gather_messages(
        start_date_override,
        end_date_override,
        duration_days_override,
    )
    .await?;
    summarize_discussion(client, api_key, model).await?;
    Ok(())
}

// ── zulip topic subcommand ────────────────────────────────────────────────────

fn resolve_topic_date_range(
    start_date_override: Option<&str>,
    end_date_override: Option<&str>,
    duration_days_override: Option<u64>,
) -> AppResult<(i64, i64, String)> {
    if let Some(duration_days) = duration_days_override {
        let now = Utc::now();
        let today = now.date_naive();
        let start_day = today
            .checked_sub_days(Days::new(duration_days))
            .ok_or("Could not compute start date from --duration-days.")?;
        let start_dt = start_day
            .and_hms_opt(0, 0, 0)
            .ok_or("Could not build start datetime at midnight UTC.")?;
        let desc = format!(
            "last {} day(s) since {} 00:00:00 UTC",
            duration_days,
            start_day.format("%Y-%m-%d")
        );
        return Ok((start_dt.and_utc().timestamp(), now.timestamp(), desc));
    }

    if start_date_override.is_some() || end_date_override.is_some() {
        let today = Utc::now().date_naive();
        let tomorrow = today
            .checked_add_days(Days::new(1))
            .ok_or("Could not compute tomorrow's date.")?
            .format("%Y-%m-%d")
            .to_string();
        let start_str = start_date_override.unwrap_or("2000-01-01");
        let end_str = end_date_override.unwrap_or(tomorrow.as_str());
        let (start_ts, end_ts) = parse_utc_range(start_str, end_str)?;
        let desc = format!("from {} to {}", start_str, end_str);
        return Ok((start_ts, end_ts, desc));
    }

    // Default: fetch all history from the beginning of the topic.
    Ok((0, Utc::now().timestamp(), "all history".to_string()))
}

async fn fetch_topic_messages(
    base_url: &str,
    email: &str,
    zulip_api_key: &str,
    channel: &str,
    topic: &str,
    start_ts: i64,
    end_ts: i64,
) -> AppResult<Vec<ZulipOutputMessage>> {
    let normalized_base = base_url.trim_end_matches('/');
    let messages_url = if normalized_base.ends_with("/api/v1") {
        format!("{normalized_base}/messages")
    } else {
        format!("{normalized_base}/api/v1/messages")
    };

    let narrow = serde_json::json!([
        {"operator": "stream", "operand": channel},
        {"operator": "topic",  "operand": topic}
    ])
    .to_string();

    let http = Client::new();
    let mut anchor = "newest".to_string();
    let mut collected: Vec<ZulipOutputMessage> = Vec::new();

    loop {
        let page_size = ZULIP_PAGE_SIZE.to_string();
        let response = http
            .get(&messages_url)
            .basic_auth(email, Some(zulip_api_key))
            .query(&[
                ("anchor", anchor.as_str()),
                ("num_before", page_size.as_str()),
                ("num_after", "0"),
                ("apply_markdown", "false"),
                ("narrow", narrow.as_str()),
            ])
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(format!("Zulip API error {status}: {error_body}").into());
        }

        let page = response.json::<ZulipMessagesResponse>().await?;
        if page.messages.is_empty() {
            break;
        }

        let oldest_id = page.messages.first().map(|m| m.id).unwrap_or(0);
        let oldest_ts = page
            .messages
            .first()
            .map(|m| m.timestamp)
            .unwrap_or(i64::MAX);

        for message in page.messages {
            if message.timestamp < start_ts || message.timestamp >= end_ts {
                continue;
            }
            let topic_text = if !message.topic.is_empty() {
                message.topic
            } else {
                message.subject
            };
            collected.push(ZulipOutputMessage {
                id: message.id,
                timestamp: message.timestamp,
                datetime: timestamp_to_rfc3339(message.timestamp),
                stream: display_recipient_to_stream_name(&message.display_recipient),
                topic: topic_text,
                sender_full_name: message.sender_full_name,
                sender_email: message.sender_email,
                content: message.content,
            });
        }

        if page.found_oldest || oldest_id == 0 || oldest_ts < start_ts {
            break;
        }

        anchor = oldest_id.saturating_sub(1).to_string();
    }

    collected.sort_by_key(|m| (m.timestamp, m.id));
    Ok(collected)
}

async fn summarize_topic(
    client: &Client,
    api_key: &str,
    model: &str,
    messages: Vec<ZulipOutputMessage>,
    channel: &str,
    topic: &str,
) -> AppResult<()> {
    let chunks = build_zulip_topic_chunks(messages, ZULIP_SUMMARY_CHUNK_SIZE);
    println!(
        "Summarizing {} chunk(s) for channel=\"{}\" topic=\"{}\"...",
        chunks.len(),
        channel,
        topic
    );

    fs::create_dir_all("./outputs")?;

    if chunks.len() == 1 {
        let scope = chunk_scope(&chunks[0]);
        let summary =
            ask_chatgpt_to_summarize_zulip_chunk(client, api_key, model, &scope, &chunks[0])
                .await?;
        fs::write(
            ZULIP_TOPIC_SUMMARY_PATH,
            format!("{}\n", summary.trim_end()),
        )?;
    } else {
        let mut partial_summaries: Vec<String> = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            let scope = chunk_scope(chunk);
            println!(
                "Summarizing chunk {}/{} ({})...",
                index + 1,
                chunks.len(),
                scope
            );
            match ask_chatgpt_to_summarize_zulip_chunk(client, api_key, model, &scope, chunk)
                .await
            {
                Ok(result) => {
                    let trimmed = result.trim_end();
                    partial_summaries
                        .push(format!("CHUNK_{} [{}]:\n{trimmed}", index + 1, scope));
                }
                Err(err) => {
                    eprintln!("Chunk {} failed: {err}", index + 1);
                }
            }
        }

        if partial_summaries.is_empty() {
            return Err("All topic summary chunks failed.".into());
        }

        let merged = partial_summaries.join("\n\n");
        let final_summary =
            ask_chatgpt_to_merge_zulip_summaries(client, api_key, model, &merged).await?;
        fs::write(
            ZULIP_TOPIC_SUMMARY_PATH,
            format!("{}\n", final_summary.trim_end()),
        )?;
    }

    println!("Topic summary done. Wrote: {ZULIP_TOPIC_SUMMARY_PATH}");
    Ok(())
}

pub async fn run_topic_report(
    client: &Client,
    api_key: &str,
    model: &str,
    channel: &str,
    topic: &str,
    start_date_override: Option<&str>,
    end_date_override: Option<&str>,
    duration_days_override: Option<u64>,
) -> AppResult<()> {
    let zulip_base_url = env::var("ZULIP_BASE_URL")
        .map_err(|_| "ZULIP_BASE_URL is required. Example: https://your-org.zulipchat.com")?;
    let zulip_email =
        env::var("ZULIP_EMAIL").map_err(|_| "ZULIP_EMAIL is required for Zulip authentication.")?;
    let zulip_api_key = env::var("ZULIP_API_KEY")
        .map_err(|_| "ZULIP_API_KEY is required for Zulip authentication.")?;

    let (start_ts, end_ts, date_desc) =
        resolve_topic_date_range(start_date_override, end_date_override, duration_days_override)?;

    println!(
        "Fetching Zulip topic: channel=\"{}\" topic=\"{}\" ({})",
        channel, topic, date_desc
    );

    let messages = fetch_topic_messages(
        &zulip_base_url,
        &zulip_email,
        &zulip_api_key,
        channel,
        topic,
        start_ts,
        end_ts,
    )
    .await?;

    if messages.is_empty() {
        eprintln!(
            "No messages found for channel=\"{}\" topic=\"{}\" in the given date range.",
            channel, topic
        );
        return Ok(());
    }

    println!("Fetched {} messages.", messages.len());

    fs::create_dir_all("./inputs")?;
    let json = serde_json::to_string_pretty(&messages)?;
    fs::write(ZULIP_TOPIC_INPUT_PATH, format!("{json}\n"))?;

    summarize_topic(client, api_key, model, messages, channel, topic).await?;
    Ok(())
}
