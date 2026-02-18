use chrono::{DateTime, Days, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::Command;

const BATCH_SIZE: usize = 10;
const INPUT_JSON_PATH: &str = "./inputs/input.json";
const OUTPUT_ITEMS_PATH: &str = "./outputs/items.txt";
const OUTPUT_RESULT_PATH: &str = "./outputs/result.txt";
const OUTPUT_RESULTS_PATH: &str = "./outputs/results.txt";
const ZULIP_OUTPUT_PATH: &str = "./inputs/zulip_messages.json";
const ZULIP_CHUNK_SUMMARIES_PATH: &str = "./outputs/zulip_chunk_summaries.txt";
const ZULIP_SUMMARY_PATH: &str = "./outputs/zulip_summary.md";
const OPENAI_CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";
const DEFAULT_ORG: &str = "hoprnet";
const DEFAULT_START_DATE: &str = "2026-01-19";
const DEFAULT_END_DATE: &str = "2026-02-11";
const ZULIP_PAGE_SIZE: usize = 500;
const ZULIP_SUMMARY_CHUNK_SIZE: usize = 80;
const ZULIP_MESSAGE_CHAR_LIMIT: usize = 700;

type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Deserialize)]
struct InputItem {
    url: Option<String>,
    title: Option<String>,
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhRepo {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
    #[serde(rename = "isPrivate")]
    is_private: bool,
}

#[derive(Debug, Deserialize)]
struct GhAuthor {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GhPr {
    author: Option<GhAuthor>,
    body: Option<String>,
    state: String,
    title: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    url: String,
}

#[derive(Debug, Serialize)]
struct GatherOutputPr {
    author: String,
    body: Option<String>,
    state: String,
    title: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    url: String,
}

#[derive(Debug, Serialize)]
struct ChatCompletionsRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChatChoiceMessage {
    content: String,
}

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

fn env_or_default(keys: &[&str], default: &str) -> String {
    keys.iter()
        .find_map(|key| env::var(key).ok().filter(|value| !value.trim().is_empty()))
        .unwrap_or_else(|| default.to_string())
}

fn pick_date(override_date: Option<&str>, env_keys: &[&str], fallback_default: &str) -> String {
    override_date
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| env_or_default(env_keys, fallback_default))
}

fn parse_utc_range(start_date: &str, end_date: &str) -> AppResult<(i64, i64)> {
    let start = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
        .map_err(|_| format!("Invalid start date `{start_date}`. Expected format YYYY-MM-DD."))?;
    let end = NaiveDate::parse_from_str(end_date, "%Y-%m-%d")
        .map_err(|_| format!("Invalid end date `{end_date}`. Expected format YYYY-MM-DD."))?;

    if end < start {
        return Err(
            format!("Invalid date range: end date `{end_date}` is before `{start_date}`.").into(),
        );
    }

    let start_dt = start
        .and_hms_opt(0, 0, 0)
        .ok_or("Could not build start datetime at midnight UTC.")?;
    let end_exclusive = end
        .checked_add_days(Days::new(1))
        .ok_or("Could not compute exclusive end datetime.")?
        .and_hms_opt(0, 0, 0)
        .ok_or("Could not build end datetime at midnight UTC.")?;

    Ok((
        start_dt.and_utc().timestamp(),
        end_exclusive.and_utc().timestamp(),
    ))
}

#[derive(Debug, Clone, Copy)]
enum CliCommand {
    Github,
    Zulip,
    Help,
}

#[derive(Debug)]
struct CliArgs<'a> {
    command: CliCommand,
    start_date: Option<&'a str>,
    end_date: Option<&'a str>,
}

fn display_recipient_to_stream_name(value: &serde_json::Value) -> String {
    if let Some(name) = value.as_str() {
        return name.to_string();
    }
    value
        .as_object()
        .and_then(|obj| obj.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or_default()
        .to_string()
}

fn timestamp_to_rfc3339(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

fn run_command(program: &str, args: &[&str]) -> AppResult<String> {
    let output = Command::new(program).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "command failed: {} {}\n{}",
            program,
            args.join(" "),
            stderr.trim()
        )
        .into());
    }

    Ok(String::from_utf8(output.stdout)?)
}

fn gather_inputs(
    start_date_override: Option<&str>,
    end_date_override: Option<&str>,
) -> AppResult<()> {
    let org = env_or_default(&["GITHUB_ORG"], DEFAULT_ORG);
    let start_date = pick_date(
        start_date_override,
        &["START_DATE", "start_date"],
        DEFAULT_START_DATE,
    );
    let end_date = pick_date(
        end_date_override,
        &["END_DATE", "end_date"],
        DEFAULT_END_DATE,
    );

    println!(
        "Fetching PRs from {} to {} across public repos in org: {}",
        start_date, end_date, org
    );

    let search = format!(
        "merged:>={} updated:<{} -author:app/renovate label:stale -label:stale -draft:true",
        start_date, end_date
    );

    let repo_output = run_command(
        "gh",
        &[
            "repo",
            "list",
            &org,
            "--limit",
            "1000",
            "--json",
            "nameWithOwner,isPrivate",
        ],
    )?;

    let repos: Vec<GhRepo> = serde_json::from_str(&repo_output)?;
    let public_repos: Vec<String> = repos
        .into_iter()
        .filter(|repo| !repo.is_private)
        .map(|repo| repo.name_with_owner)
        .collect();

    let mut all_prs: Vec<GatherOutputPr> = Vec::new();
    for repo in &public_repos {
        println!("-> {repo}");

        let prs_output = run_command(
            "gh",
            &[
                "pr",
                "list",
                "--repo",
                repo,
                "--search",
                &search,
                "-s",
                "all",
                "-L",
                "200",
                "--json",
                "state,author,title,body,url,updatedAt",
            ],
        )?;

        let prs: Vec<GhPr> = serde_json::from_str(&prs_output)?;
        all_prs.extend(prs.into_iter().map(|pr| GatherOutputPr {
            author: pr.author.map(|author| author.login).unwrap_or_default(),
            body: pr.body,
            state: pr.state,
            title: pr.title,
            updated_at: pr.updated_at,
            url: pr.url,
        }));
    }

    fs::create_dir_all("./inputs")?;
    let json = serde_json::to_string_pretty(&all_prs)?;
    fs::write(INPUT_JSON_PATH, format!("{json}\n"))?;

    println!(
        "Gather done. Wrote {} PRs to {}",
        all_prs.len(),
        INPUT_JSON_PATH
    );
    Ok(())
}

async fn gather_zulip_messages(
    start_date_override: Option<&str>,
    end_date_override: Option<&str>,
) -> AppResult<()> {
    let base_url = env::var("ZULIP_BASE_URL")
        .map_err(|_| "ZULIP_BASE_URL is required. Example: https://your-org.zulipchat.com")?;
    let email =
        env::var("ZULIP_EMAIL").map_err(|_| "ZULIP_EMAIL is required for Zulip authentication.")?;
    let api_key = env::var("ZULIP_API_KEY")
        .map_err(|_| "ZULIP_API_KEY is required for Zulip authentication.")?;

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

    println!(
        "Fetching Zulip stream messages from {} to {} (UTC) across all channels/topics",
        start_date, end_date
    );

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

fn compact_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut chars = input.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn build_batch_prompt(input_array: &[InputItem]) -> String {
    let mut content = String::new();

    for item in input_array {
        let url = item.url.as_deref().unwrap_or("").trim();
        let title = item.title.as_deref().unwrap_or("").trim();
        let body = compact_whitespace(item.body.as_deref().unwrap_or("").trim());

        content.push_str("- PR_URL=");
        content.push_str(url);
        content.push('\n');
        content.push_str("  TITLE=");
        content.push_str(title);
        content.push('\n');
        content.push_str("  BODY=");
        content.push_str(&body);
        content.push_str("\n\n");
    }

    content
}

fn read_input_items() -> AppResult<Vec<InputItem>> {
    let raw = fs::read_to_string(INPUT_JSON_PATH)?;
    let items = serde_json::from_str::<Vec<InputItem>>(&raw)?;
    Ok(items)
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

async fn create_chat_completion(
    client: &Client,
    api_key: &str,
    model: &str,
    prompt: String,
) -> AppResult<String> {
    let request_body = ChatCompletionsRequest {
        model: model.to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }],
    };

    let response = client
        .post(OPENAI_CHAT_COMPLETIONS_URL)
        .bearer_auth(api_key)
        .json(&request_body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error {status}: {error_body}").into());
    }

    let parsed = response.json::<ChatCompletionsResponse>().await?;
    let message = parsed
        .choices
        .into_iter()
        .next()
        .map(|choice| choice.message.content)
        .unwrap_or_default();

    if message.trim().is_empty() {
        return Err("OpenAI API returned empty completion content".into());
    }

    Ok(message)
}

async fn ask_chatgpt_to_summarize(
    client: &Client,
    api_key: &str,
    model: &str,
    input_array: &[InputItem],
) -> AppResult<String> {
    let batch_content = build_batch_prompt(input_array);
    let prompt = [
        "Summarize EACH PR into ONE numbered line.",
        "Hard rules:",
        "1) No sub-bullets.",
        "2) At the end of EACH line, append the exact token `PR_URL=<...>` copied verbatim from the input.",
        "3) Do NOT shorten, omit, rewrite, or markdown the URL. Keep it exactly as provided.",
        "Format per line: `N. <summary>. PR_URL=<full_url>`",
        "",
        "Input PRs:",
        &batch_content,
    ]
    .join("\n");

    create_chat_completion(client, api_key, model, prompt).await
}

async fn ask_chatgpt_to_group(
    client: &Client,
    api_key: &str,
    model: &str,
    items_str: &str,
) -> AppResult<String> {
    let prompt = [
        "You are a technical team lead who needs to provide a tri-weekly update on development progress.",
        "Group the PRs by purpose/functionality and write a short description for each group.",
        "Hard rules:",
        "1) Number the groups.",
        "2) Order groups by importance/impact to the protocol.",
        "3) For every PR you mention, include its exact `PR_URL=<...>` token copied verbatim (do NOT shorten or rewrite URLs).",
        "4) No sub-bullets.",
        "",
        "Here are the PR summaries (each line ends with PR_URL=...):",
        items_str,
    ]
    .join("\n");

    create_chat_completion(client, api_key, model, prompt).await
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

async fn step_one(client: &Client, api_key: &str, model: &str) -> AppResult<()> {
    let input_items = read_input_items()?;
    let query_inputs: Vec<&[InputItem]> = input_items.chunks(BATCH_SIZE).collect();

    println!(
        "There are {} items in {} batches.",
        input_items.len(),
        query_inputs.len()
    );

    fs::create_dir_all("./outputs")?;
    fs::write(OUTPUT_ITEMS_PATH, "")?;

    let mut output = OpenOptions::new().append(true).open(OUTPUT_ITEMS_PATH)?;
    for (index, batch) in query_inputs.iter().enumerate() {
        println!("Generating {}/{} batches...", index + 1, query_inputs.len());
        match ask_chatgpt_to_summarize(client, api_key, model, batch).await {
            Ok(result) => {
                writeln!(output, "{}", result.trim_end())?;
            }
            Err(err) => {
                eprintln!("Batch {} failed: {err}", index + 1);
            }
        }
    }

    println!("Step 1 done. Wrote: {OUTPUT_ITEMS_PATH}");
    Ok(())
}

async fn step_two(client: &Client, api_key: &str, model: &str) -> AppResult<()> {
    println!("Starting Step 2...");
    fs::create_dir_all("./outputs")?;
    fs::write(OUTPUT_RESULT_PATH, "")?;
    fs::write(OUTPUT_RESULTS_PATH, "")?;

    let items = fs::read_to_string(OUTPUT_ITEMS_PATH).unwrap_or_default();
    let trimmed = items.trim();
    if trimmed.is_empty() {
        eprintln!("No items found in outputs/items.txt. Run step 1 first.");
        return Ok(());
    }

    let result = ask_chatgpt_to_group(client, api_key, model, trimmed).await?;
    let mut output = OpenOptions::new().append(true).open(OUTPUT_RESULT_PATH)?;
    let mut output_compat = OpenOptions::new().append(true).open(OUTPUT_RESULTS_PATH)?;
    writeln!(output, "{}", result.trim_end())?;
    writeln!(output_compat, "{}", result.trim_end())?;

    println!("Step 2 done. Wrote: {OUTPUT_RESULT_PATH} and {OUTPUT_RESULTS_PATH}");
    Ok(())
}

async fn summarize_zulip_discussion(client: &Client, api_key: &str, model: &str) -> AppResult<()> {
    println!("Starting Zulip discussion summary...");
    let messages = read_zulip_messages()?;
    if messages.is_empty() {
        eprintln!("No messages found in {ZULIP_OUTPUT_PATH}. Run gather-zulip first.");
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

fn parse_cli_args(args: &[String]) -> AppResult<CliArgs<'_>> {
    let mut command = CliCommand::Github;
    let mut index = 1usize;

    if let Some(first) = args.get(1).map(String::as_str) {
        match first {
            "github" => {
                command = CliCommand::Github;
                index = 2;
            }
            "zulip" => {
                command = CliCommand::Zulip;
                index = 2;
            }
            "help" | "-h" | "--help" => {
                return Ok(CliArgs {
                    command: CliCommand::Help,
                    start_date: None,
                    end_date: None,
                });
            }
            "--start-date" | "--end-date" => {
                command = CliCommand::Github;
                index = 1;
            }
            other if other.starts_with('-') => {
                return Err(format!("Unknown argument `{other}`. Use `--help` for usage.").into());
            }
            other => {
                return Err(format!(
                    "Unknown command `{other}`. Use `github`, `zulip`, or `--help`."
                )
                .into());
            }
        }
    }

    let mut start_date: Option<&str> = None;
    let mut end_date: Option<&str> = None;
    let mut i = index;

    while i < args.len() {
        match args[i].as_str() {
            "--start-date" => {
                if start_date.is_some() {
                    return Err("Duplicate `--start-date` argument.".into());
                }
                let value = args
                    .get(i + 1)
                    .ok_or("Missing value for `--start-date`. Expected YYYY-MM-DD.")?;
                start_date = Some(value.as_str());
                i += 2;
            }
            "--end-date" => {
                if end_date.is_some() {
                    return Err("Duplicate `--end-date` argument.".into());
                }
                let value = args
                    .get(i + 1)
                    .ok_or("Missing value for `--end-date`. Expected YYYY-MM-DD.")?;
                end_date = Some(value.as_str());
                i += 2;
            }
            "-h" | "--help" => {
                return Ok(CliArgs {
                    command: CliCommand::Help,
                    start_date: None,
                    end_date: None,
                });
            }
            other => {
                return Err(format!("Unknown argument `{other}`. Use `--help` for usage.").into());
            }
        }
    }

    Ok(CliArgs {
        command,
        start_date,
        end_date,
    })
}

fn print_usage(bin_name: &str) {
    println!("Usage:");
    println!("  {bin_name} github [--start-date YYYY-MM-DD] [--end-date YYYY-MM-DD]");
    println!("  {bin_name} zulip  [--start-date YYYY-MM-DD] [--end-date YYYY-MM-DD]");
    println!("  {bin_name} [--start-date YYYY-MM-DD] [--end-date YYYY-MM-DD]  # defaults to github");
}

async fn run_github_report(
    client: &Client,
    api_key: &str,
    model: &str,
    start_date_override: Option<&str>,
    end_date_override: Option<&str>,
) -> AppResult<()> {
    gather_inputs(start_date_override, end_date_override)?;
    step_one(client, api_key, model).await?;
    step_two(client, api_key, model).await?;
    Ok(())
}

async fn run_zulip_report(
    client: &Client,
    api_key: &str,
    model: &str,
    start_date_override: Option<&str>,
    end_date_override: Option<&str>,
) -> AppResult<()> {
    gather_zulip_messages(start_date_override, end_date_override).await?;
    summarize_zulip_discussion(client, api_key, model).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> AppResult<()> {
    dotenvy::dotenv().ok();

    let args: Vec<String> = env::args().collect();
    let cli_args = parse_cli_args(&args)?;

    if matches!(cli_args.command, CliCommand::Help) {
        print_usage(args.first().map(String::as_str).unwrap_or("hopr-pm"));
        return Ok(());
    }

    let api_key = env::var("OPENAI_API_KEY")
        .map_err(|_| "OPENAI_API_KEY is required. Add it to your shell env or .env file.")?;
    let model = env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-5".to_string());
    let client = Client::new();

    match cli_args.command {
        CliCommand::Github => {
            run_github_report(
                &client,
                &api_key,
                &model,
                cli_args.start_date,
                cli_args.end_date,
            )
            .await?
        }
        CliCommand::Zulip => {
            run_zulip_report(
                &client,
                &api_key,
                &model,
                cli_args.start_date,
                cli_args.end_date,
            )
            .await?
        }
        CliCommand::Help => {}
    }

    Ok(())
}
