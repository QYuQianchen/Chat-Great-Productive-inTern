# hopr-pm

CLI tool for generating development activity reports from GitHub and Zulip.

## Prerequisites

- [Rust](https://rustup.rs/) (for building)
- [GitHub CLI (`gh`)](https://cli.github.com/) authenticated with the relevant orgs

## Build & install

Build a release binary and install it to `~/.cargo/bin` so `hopr-pm` is available anywhere in your shell:

```sh
cargo install --path app
```

To update after pulling changes, run the same command again.

Alternatively, build without installing (binary lands at `target/release/hopr-pm`):

```sh
cargo build --release
```

## Configuration

Copy `.env.example` to `.env` and fill in the required values:

```sh
cp .env.example .env
```

| Variable | Required | Default | Description |
|---|---|---|---|
| `OPENAI_API_KEY` | yes | — | OpenAI API key |
| `OPENAI_MODEL` | no | `gpt-5` | Model to use |
| `GITHUB_ORG` | no | `hoprnet` | Org for the `github` command |
| `START_DATE` | no | see `.env.example` | Default start date (`YYYY-MM-DD`) |
| `END_DATE` | no | see `.env.example` | Default end date (`YYYY-MM-DD`) |
| `ZULIP_BASE_URL` | zulip only | — | e.g. `https://your-org.zulipchat.com` |
| `ZULIP_EMAIL` | zulip only | — | Zulip bot/user email |
| `ZULIP_API_KEY` | zulip only | — | Zulip API key |
| `ZULIP_START_DATE` | no | — | Start date for Zulip fetch |
| `ZULIP_END_DATE` | no | — | End date for Zulip fetch |

## Commands

### `github` (default)

Fetches merged PRs from all public repos in `GITHUB_ORG` and produces a grouped development summary.

```sh
hopr-pm github --start-date 2026-01-01 --end-date 2026-03-31
hopr-pm github --duration-days 14
hopr-pm                                  # defaults to github
```

Outputs: `inputs/input.json`, `outputs/items.txt`, `outputs/result.txt`, `outputs/results.txt`

### `gnosis-vpn`

Fetches merged PRs from a fixed set of GnosisVPN repositories across two orgs and produces a quarterly report categorized by Features, Bug Fixes, Infrastructure, Documentation, and Other.

Repos covered:
- **gnosis**: `gnosis_vpn-client`, `gnosis_vpn-app`, `gnosis_vpn`, `gnosis_vpn-website`, `gnosis_vpn-server`, `gnosis_vpn-downloads_website`, `gnosis_vpn-self-onboarding`
- **hoprnet**: `hoprnet`, `blokli`, `edge-client`

```sh
hopr-pm gnosis-vpn --start-date 2026-01-01 --end-date 2026-03-31
hopr-pm gnosis-vpn --duration-days 90
```

Outputs: `inputs/gnosis_vpn_input.json`, `outputs/gnosis_vpn_items.txt`, `outputs/gnosis_vpn_result.txt`

### `zulip`

Fetches messages from Zulip and produces a summarized report grouped by stream/topic.

```sh
hopr-pm zulip --start-date 2026-01-01 --end-date 2026-03-31
hopr-pm zulip --duration-days 14
```

Outputs: `inputs/zulip_messages.json`, `outputs/zulip_chunk_summaries.txt`, `outputs/zulip_summary.md`

### `zulip topic`

Deep-dives into a single channel/topic thread. Fetches only that thread's messages and produces a focused summary.

```sh
# All history (default when no date flags are given)
hopr-pm zulip topic --channel my-channel --topic "some topic"

# Explicit date range
hopr-pm zulip topic --channel my-channel --topic "some topic" \
  --start-date 2026-01-01 --end-date 2026-03-31

# Lookback
hopr-pm zulip topic --channel my-channel --topic "some topic" --duration-days 90
```

`--channel` and `--topic` are required. When no date flags are given, all history for that thread is fetched.

Outputs: `inputs/zulip_topic_messages.json`, `outputs/zulip_topic_summary.md`

## Date options

All commands accept these flags (mutually exclusive):

| Flag | Description |
|---|---|
| `--start-date YYYY-MM-DD` | Explicit start date |
| `--end-date YYYY-MM-DD` | Explicit end date |
| `--duration-days N` | Lookback N days from today at `00:00:00 UTC`; cannot combine with the above |

**Default behaviour when no date flags are given:**
- `github`, `gnosis-vpn`, `zulip` — fall back to `START_DATE` / `END_DATE` env vars, then to the hardcoded defaults in `.env.example`
- `zulip topic` — fetches **all history** for that thread (no env var fallback)

## Help

```sh
hopr-pm --help
```
