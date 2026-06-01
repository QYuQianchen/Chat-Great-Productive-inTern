use chrono::{Days, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;

use crate::constants::{
    AppResult, BATCH_SIZE, DEFAULT_END_DATE, DEFAULT_ORG, DEFAULT_START_DATE, INPUT_JSON_PATH,
    OUTPUT_ITEMS_PATH, OUTPUT_RESULT_PATH, OUTPUT_RESULTS_PATH,
};
use crate::openai::create_chat_completion;
use crate::util::{compact_whitespace, env_or_default, pick_date, run_command};

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

pub fn gather_inputs(
    start_date_override: Option<&str>,
    end_date_override: Option<&str>,
    duration_days_override: Option<u64>,
) -> AppResult<()> {
    let org = env_or_default(&["GITHUB_ORG"], DEFAULT_ORG);
    let (start_date, end_date) = if let Some(duration_days) = duration_days_override {
        let today = Utc::now().date_naive();
        let start_day = today
            .checked_sub_days(Days::new(duration_days))
            .ok_or("Could not compute start date from --duration-days.")?;
        let end_day_exclusive = today
            .checked_add_days(Days::new(1))
            .ok_or("Could not compute end date from current day.")?;
        (
            start_day.format("%Y-%m-%d").to_string(),
            end_day_exclusive.format("%Y-%m-%d").to_string(),
        )
    } else {
        (
            pick_date(
                start_date_override,
                &["START_DATE", "start_date"],
                DEFAULT_START_DATE,
            ),
            pick_date(
                end_date_override,
                &["END_DATE", "end_date"],
                DEFAULT_END_DATE,
            ),
        )
    };

    if let Some(duration_days) = duration_days_override {
        println!(
            "Fetching PRs since {} 00:00:00 UTC (lookback {} day(s)) across public repos in org: {}",
            start_date, duration_days, org
        );
    } else {
        println!(
            "Fetching PRs from {} to {} across public repos in org: {}",
            start_date, end_date, org
        );
    }

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

pub async fn run_report(
    client: &Client,
    api_key: &str,
    model: &str,
    start_date_override: Option<&str>,
    end_date_override: Option<&str>,
    duration_days_override: Option<u64>,
) -> AppResult<()> {
    gather_inputs(
        start_date_override,
        end_date_override,
        duration_days_override,
    )?;
    step_one(client, api_key, model).await?;
    step_two(client, api_key, model).await?;
    Ok(())
}
