Add OpenAI API secret to `.env`:
- `OPENAI_API_KEY=...`
- optional: `OPENAI_MODEL=gpt-5`
- optional gather config: `GITHUB_ORG`, `START_DATE`, `END_DATE`

Run the full Rust flow (gather + summarize + group):
- `cargo run -p app --`

Run a single step:
- gather only: `cargo run -p app -- -s gather`
- step 1: `cargo run -p app -- -s 1`
- step 2: `cargo run -p app -- -s 2`

The app overwrites `inputs/input.json`, `outputs/items.txt`, `outputs/result.txt`, and `outputs/results.txt` on each run, so manual cleanup is not required.
