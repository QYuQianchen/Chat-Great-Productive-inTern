use crate::constants::AppResult;

#[derive(Debug, Clone)]
pub enum CliCommand {
    Github,
    GnosisVpn,
    Zulip,
    ZulipTopic { channel: String, topic: String },
    Help,
}

#[derive(Debug)]
pub struct CliArgs {
    pub command: CliCommand,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub duration_days: Option<u64>,
}

pub fn parse_cli_args(args: &[String]) -> AppResult<CliArgs> {
    let mut command = CliCommand::Github;
    let mut is_zulip_topic = false;
    let mut index = 1usize;

    if let Some(first) = args.get(1).map(String::as_str) {
        match first {
            "github" => {
                command = CliCommand::Github;
                index = 2;
            }
            "gnosis-vpn" => {
                command = CliCommand::GnosisVpn;
                index = 2;
            }
            "zulip" => {
                if args.get(2).map(String::as_str) == Some("topic") {
                    is_zulip_topic = true;
                    index = 3;
                } else {
                    command = CliCommand::Zulip;
                    index = 2;
                }
            }
            "help" | "-h" | "--help" => {
                return Ok(CliArgs {
                    command: CliCommand::Help,
                    start_date: None,
                    end_date: None,
                    duration_days: None,
                });
            }
            "--start-date" | "--end-date" | "--duration-days" => {
                command = CliCommand::Github;
                index = 1;
            }
            other if other.starts_with('-') => {
                return Err(format!("Unknown argument `{other}`. Use `--help` for usage.").into());
            }
            other => {
                return Err(format!(
                    "Unknown command `{other}`. Use `github`, `gnosis-vpn`, `zulip`, `zulip topic`, or `--help`."
                )
                .into());
            }
        }
    }

    let mut start_date: Option<String> = None;
    let mut end_date: Option<String> = None;
    let mut duration_days: Option<u64> = None;
    let mut channel: Option<String> = None;
    let mut topic_flag: Option<String> = None;
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
                start_date = Some(value.clone());
                i += 2;
            }
            "--end-date" => {
                if end_date.is_some() {
                    return Err("Duplicate `--end-date` argument.".into());
                }
                let value = args
                    .get(i + 1)
                    .ok_or("Missing value for `--end-date`. Expected YYYY-MM-DD.")?;
                end_date = Some(value.clone());
                i += 2;
            }
            "--duration-days" => {
                if duration_days.is_some() {
                    return Err("Duplicate `--duration-days` argument.".into());
                }
                let value = args.get(i + 1).ok_or(
                    "Missing value for `--duration-days`. Expected a non-negative integer.",
                )?;
                let parsed = value.parse::<u64>().map_err(|_| {
                    format!(
                        "Invalid `--duration-days` value `{}`. Expected a non-negative integer.",
                        value
                    )
                })?;
                duration_days = Some(parsed);
                i += 2;
            }
            "--channel" => {
                if !is_zulip_topic {
                    return Err(
                        "`--channel` is only valid for the `zulip topic` subcommand.".into(),
                    );
                }
                if channel.is_some() {
                    return Err("Duplicate `--channel` argument.".into());
                }
                let value = args
                    .get(i + 1)
                    .ok_or("Missing value for `--channel`. Expected a channel name.")?;
                channel = Some(value.clone());
                i += 2;
            }
            "--topic" => {
                if !is_zulip_topic {
                    return Err(
                        "`--topic` is only valid for the `zulip topic` subcommand.".into(),
                    );
                }
                if topic_flag.is_some() {
                    return Err("Duplicate `--topic` argument.".into());
                }
                let value = args
                    .get(i + 1)
                    .ok_or("Missing value for `--topic`. Expected a topic name.")?;
                topic_flag = Some(value.clone());
                i += 2;
            }
            "-h" | "--help" => {
                return Ok(CliArgs {
                    command: CliCommand::Help,
                    start_date: None,
                    end_date: None,
                    duration_days: None,
                });
            }
            other => {
                return Err(format!("Unknown argument `{other}`. Use `--help` for usage.").into());
            }
        }
    }

    if duration_days.is_some() && (start_date.is_some() || end_date.is_some()) {
        return Err(
            "`--duration-days` cannot be combined with `--start-date` or `--end-date`.".into(),
        );
    }

    let command = if is_zulip_topic {
        let ch = channel.ok_or("`zulip topic` requires `--channel <name>`.")?;
        let tp = topic_flag.ok_or("`zulip topic` requires `--topic <name>`.")?;
        CliCommand::ZulipTopic {
            channel: ch,
            topic: tp,
        }
    } else {
        command
    };

    Ok(CliArgs {
        command,
        start_date,
        end_date,
        duration_days,
    })
}

pub fn print_usage(bin_name: &str) {
    println!("Usage:");
    println!(
        "  {bin_name} github       [--start-date YYYY-MM-DD] [--end-date YYYY-MM-DD] [--duration-days N]"
    );
    println!(
        "  {bin_name} gnosis-vpn   [--start-date YYYY-MM-DD] [--end-date YYYY-MM-DD] [--duration-days N]"
    );
    println!(
        "  {bin_name} zulip        [--start-date YYYY-MM-DD] [--end-date YYYY-MM-DD] [--duration-days N]"
    );
    println!(
        "  {bin_name} zulip topic  --channel <name> --topic <name> [--start-date YYYY-MM-DD] [--end-date YYYY-MM-DD] [--duration-days N]"
    );
    println!(
        "  {bin_name} [--start-date YYYY-MM-DD] [--end-date YYYY-MM-DD] [--duration-days N]  # defaults to github"
    );
    println!(
        "  Note: --duration-days cannot be combined with --start-date/--end-date. Start time is 00:00:00 UTC."
    );
    println!(
        "  Note: `zulip topic` defaults to all history when no date flags are given."
    );
}
