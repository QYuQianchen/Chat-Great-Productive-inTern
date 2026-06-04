use chrono::{Days, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;

use crate::constants::{
    AppResult, BATCH_SIZE, DEFAULT_END_DATE, DEFAULT_START_DATE, GNOSIS_VPN_INPUT_JSON_PATH,
    GNOSIS_VPN_OUTPUT_ITEMS_PATH, GNOSIS_VPN_OUTPUT_RESULT_PATH,
};
use crate::openai::create_chat_completion;
use crate::util::{compact_whitespace, pick_date, run_command};

const REPOS: &[(&str, &str)] = &[
    ("gnosis", "gnosis_vpn-client"),
    ("gnosis", "gnosis_vpn-app"),
    ("gnosis", "gnosis_vpn"),
    ("gnosis", "gnosis_vpn-website"),
    ("gnosis", "gnosis_vpn-server"),
    ("gnosis", "gnosis_vpn-downloads_website"),
    ("gnosis", "gnosis_vpn-self-onboarding"),
    ("hoprnet", "hoprnet"),
    ("hoprnet", "blokli"),
    ("hoprnet", "edge-client"),
];

#[derive(Debug, Deserialize)]
struct InputItem {
    url: Option<String>,
    title: Option<String>,
    body: Option<String>,
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
            "Fetching GnosisVPN PRs since {} 00:00:00 UTC (lookback {} day(s)) across {} repos",
            start_date,
            duration_days,
            REPOS.len()
        );
    } else {
        println!(
            "Fetching GnosisVPN PRs from {} to {} across {} repos",
            start_date,
            end_date,
            REPOS.len()
        );
    }

    let search = format!(
        "merged:>={} updated:<{} -author:app/renovate -label:stale -draft:true",
        start_date, end_date
    );

    let mut all_prs: Vec<GatherOutputPr> = Vec::new();
    for (org, repo) in REPOS {
        let full_repo = format!("{org}/{repo}");
        print!("-> {full_repo} ... ");
        std::io::stdout().flush().ok();

        match run_command(
            "gh",
            &[
                "pr",
                "list",
                "--repo",
                &full_repo,
                "--search",
                &search,
                "-s",
                "all",
                "-L",
                "500",
                "--json",
                "state,author,title,body,url,updatedAt",
            ],
        ) {
            Ok(prs_output) => match serde_json::from_str::<Vec<GhPr>>(&prs_output) {
                Ok(prs) => {
                    let count = prs.len();
                    all_prs.extend(prs.into_iter().map(|pr| GatherOutputPr {
                        author: pr.author.map(|a| a.login).unwrap_or_default(),
                        body: pr.body,
                        state: pr.state,
                        title: pr.title,
                        updated_at: pr.updated_at,
                        url: pr.url,
                    }));
                    println!("{count} PRs");
                }
                Err(err) => eprintln!("parse error: {err}"),
            },
            Err(err) => eprintln!("skipped ({})", err.to_string().lines().next().unwrap_or("")),
        }
    }

    fs::create_dir_all("./inputs")?;
    let json = serde_json::to_string_pretty(&all_prs)?;
    fs::write(GNOSIS_VPN_INPUT_JSON_PATH, format!("{json}\n"))?;

    println!(
        "Gather done. Wrote {} PRs to {}",
        all_prs.len(),
        GNOSIS_VPN_INPUT_JSON_PATH
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
    let raw = fs::read_to_string(GNOSIS_VPN_INPUT_JSON_PATH)?;
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
        "You are a technical program manager writing a quarterly development report for GnosisVPN.",
        "Analyze the following PR summaries and organize them into these sections:",
        "1. Features & New Functionality — new capabilities, user-facing improvements, protocol changes",
        "2. Bug Fixes — defect corrections, reliability improvements, regression fixes",
        "3. Infrastructure & DevOps — CI/CD, build tooling, dependency updates, configuration, testing infrastructure",
        "4. Documentation — docs, changelogs, guides, README updates",
        "5. Other — anything that does not fit the above categories",
        "Hard rules:",
        "1) Use those exact section headings, each preceded by its number.",
        "2) For each non-empty section, write a 1-2 sentence summary of the overall activity, then list the relevant PRs.",
        "3) Order PRs within each section by importance/impact.",
        "4) For every PR you mention, include its exact `PR_URL=<...>` token copied verbatim (do NOT shorten or rewrite URLs).",
        "5) No sub-bullets.",
        "6) Omit sections that have no PRs.",
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
    fs::write(GNOSIS_VPN_OUTPUT_ITEMS_PATH, "")?;

    let mut output = OpenOptions::new()
        .append(true)
        .open(GNOSIS_VPN_OUTPUT_ITEMS_PATH)?;
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

    println!("Step 1 done. Wrote: {GNOSIS_VPN_OUTPUT_ITEMS_PATH}");
    Ok(())
}

async fn step_two(client: &Client, api_key: &str, model: &str) -> AppResult<()> {
    println!("Starting Step 2...");
    fs::create_dir_all("./outputs")?;
    fs::write(GNOSIS_VPN_OUTPUT_RESULT_PATH, "")?;

    let items = fs::read_to_string(GNOSIS_VPN_OUTPUT_ITEMS_PATH).unwrap_or_default();
    let trimmed = items.trim();
    if trimmed.is_empty() {
        eprintln!(
            "No items found in {}. Run step 1 first.",
            GNOSIS_VPN_OUTPUT_ITEMS_PATH
        );
        return Ok(());
    }

    let result = ask_chatgpt_to_group(client, api_key, model, trimmed).await?;
    let mut output = OpenOptions::new()
        .append(true)
        .open(GNOSIS_VPN_OUTPUT_RESULT_PATH)?;
    writeln!(output, "{}", result.trim_end())?;

    println!("Step 2 done. Wrote: {GNOSIS_VPN_OUTPUT_RESULT_PATH}");
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
    gather_inputs(start_date_override, end_date_override, duration_days_override)?;
    step_one(client, api_key, model).await?;
    step_two(client, api_key, model).await?;
    Ok(())
}
