use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::Command;

const BATCH_SIZE: usize = 10;
const INPUT_JSON_PATH: &str = "./inputs/input.json";
const OUTPUT_ITEMS_PATH: &str = "./outputs/items.txt";
const OUTPUT_RESULT_PATH: &str = "./outputs/result.txt";
const OUTPUT_RESULTS_PATH: &str = "./outputs/results.txt";
const OPENAI_CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";
const DEFAULT_ORG: &str = "hoprnet";
const DEFAULT_START_DATE: &str = "2026-01-19";
const DEFAULT_END_DATE: &str = "2026-02-11";

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

fn env_or_default(keys: &[&str], default: &str) -> String {
    keys.iter()
        .find_map(|key| env::var(key).ok().filter(|value| !value.trim().is_empty()))
        .unwrap_or_else(|| default.to_string())
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

fn gather_inputs() -> AppResult<()> {
    let org = env_or_default(&["GITHUB_ORG"], DEFAULT_ORG);
    let start_date = env_or_default(&["START_DATE", "start_date"], DEFAULT_START_DATE);
    let end_date = env_or_default(&["END_DATE", "end_date"], DEFAULT_END_DATE);

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

fn compact_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
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

fn parse_step_arg(args: &[String]) -> Option<&str> {
    if args.len() >= 3 && (args[1] == "-s" || args[1] == "--step") {
        return Some(args[2].as_str());
    }
    None
}

#[tokio::main]
async fn main() -> AppResult<()> {
    dotenvy::dotenv().ok();

    let args: Vec<String> = env::args().collect();
    let step = parse_step_arg(&args);

    if matches!(step, Some("0") | Some("gather")) {
        gather_inputs()?;
        return Ok(());
    }

    let api_key = env::var("OPENAI_API_KEY")
        .map_err(|_| "OPENAI_API_KEY is required. Add it to your shell env or .env file.")?;
    let model = env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-5".to_string());
    let client = Client::new();

    match step {
        Some("1") | Some("one") => step_one(&client, &api_key, &model).await?,
        Some("2") | Some("two") => step_two(&client, &api_key, &model).await?,
        _ => {
            gather_inputs()?;
            step_one(&client, &api_key, &model).await?;
            step_two(&client, &api_key, &model).await?;
        }
    }

    Ok(())
}
