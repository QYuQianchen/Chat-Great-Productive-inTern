use arboard::Clipboard;
use chrono::{DateTime, NaiveDate, Utc};
use octocrab::{models::pulls::PullRequest, params::State, Octocrab};
use serde::Serialize;
use std::error::Error;

#[derive(Serialize)]
struct SimplePR {
    author: String,
    body: Option<String>,
    state: String,
    title: String,
    updated_at: DateTime<Utc>,
    url: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let repo_owner = "hoprnet";
    let repo_name = "hoprnet";

    let start_date = NaiveDate::from_ymd_opt(2025, 3, 10).unwrap();
    let end_date = NaiveDate::from_ymd_opt(2025, 4, 21).unwrap();

    println!("Fetching PRs from {} to {}", start_date, end_date);

    let query = format!(
        "repo:{}/{} is:pr merged:>={} updated:<{} -author:app/renovate -label:stale -draft:true",
        repo_owner,
        repo_name,
        start_date,
        end_date
    );

    let gh = Octocrab::builder().build()?;

    let search_result = gh
        .search()
        .issues_and_pull_requests(&query)
        .sort("updated")
        .order("desc")
        .per_page(100)
        .send()
        .await?;

    let prs: Vec<SimplePR> = search_result
        .items
        .into_iter()
        .filter(|item| item.pull_request.is_some())
        .map(|item| SimplePR {
            author: item.user.login,
            body: item.body,
            state: item.state.to_string(),
            title: item.title,
            updated_at: item.updated_at.unwrap_or_else(|| Utc::now()),
            url: item.html_url.to_string(),
        })
        .collect();

    let json = serde_json::to_string_pretty(&prs)?;
    Clipboard::new()?.set_text(json)?;

    println!("Copied {} PRs to clipboard.", prs.len());

    Ok(())
}
