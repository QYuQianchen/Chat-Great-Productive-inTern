mod cli;
mod constants;
mod github;
mod openai;
mod util;
mod zulip;

use constants::AppResult;
use reqwest::Client;
use std::env;

#[tokio::main]
async fn main() -> AppResult<()> {
    dotenvy::dotenv().ok();

    let args: Vec<String> = env::args().collect();
    let cli_args = cli::parse_cli_args(&args)?;

    if matches!(cli_args.command, cli::CliCommand::Help) {
        cli::print_usage(args.first().map(String::as_str).unwrap_or("hopr-pm"));
        return Ok(());
    }

    let api_key = env::var("OPENAI_API_KEY")
        .map_err(|_| "OPENAI_API_KEY is required. Add it to your shell env or .env file.")?;
    let model = env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-5".to_string());
    let client = Client::new();

    match cli_args.command {
        cli::CliCommand::Github => {
            github::run_report(
                &client,
                &api_key,
                &model,
                cli_args.start_date.as_deref(),
                cli_args.end_date.as_deref(),
                cli_args.duration_days,
            )
            .await?
        }
        cli::CliCommand::Zulip => {
            zulip::run_report(
                &client,
                &api_key,
                &model,
                cli_args.start_date.as_deref(),
                cli_args.end_date.as_deref(),
                cli_args.duration_days,
            )
            .await?
        }
        cli::CliCommand::Help => {}
    }

    Ok(())
}
