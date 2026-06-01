Add OpenAI API secret to `.env`:
- `OPENAI_API_KEY=...`
- optional: `OPENAI_MODEL=gpt-5`
- optional GitHub config: `GITHUB_ORG`, `START_DATE`, `END_DATE`
- optional Zulip config:
  - `ZULIP_BASE_URL=https://your-org.zulipchat.com`
  - `ZULIP_EMAIL=you@example.com`
  - `ZULIP_API_KEY=...`
  - `ZULIP_START_DATE=YYYY-MM-DD`
  - `ZULIP_END_DATE=YYYY-MM-DD`

CLI commands:
- `./hopr-pm github` to gather + summarize GitHub activities.
- `./hopr-pm zulip` to gather + summarize Zulip messages.
- `./hopr-pm` defaults to the GitHub command.
- optional date window for either command:
  - `--start-date YYYY-MM-DD`
  - `--end-date YYYY-MM-DD`
- lookback duration option for either command:
  - `--duration-days N`
  - starts at `00:00:00 UTC` on `N` days ago
  - cannot be combined with `--start-date` or `--end-date`

Examples:
- `./hopr-pm github --start-date 2026-01-01 --end-date 2026-01-31`
- `./hopr-pm zulip --start-date 2026-01-01 --end-date 2026-01-31`
- `./hopr-pm github --duration-days 2`
- `./hopr-pm zulip --duration-days 2`
- help: `./hopr-pm --help`

Equivalent Cargo form:
- `cargo run -p hopr-pm -- github`
- `cargo run -p hopr-pm -- zulip`

Outputs:
- GitHub flow writes `inputs/input.json`, `outputs/items.txt`, `outputs/result.txt`, and `outputs/results.txt`.
- Zulip flow writes `inputs/zulip_messages.json`, `outputs/zulip_chunk_summaries.txt`, and `outputs/zulip_summary.md`.
- Zulip summaries are chunked by channel/topic first (maximizing same stream/topic messages per chunk), then merged into a final report.
