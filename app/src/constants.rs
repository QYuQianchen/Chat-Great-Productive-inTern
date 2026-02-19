pub const BATCH_SIZE: usize = 10;
pub const INPUT_JSON_PATH: &str = "./inputs/input.json";
pub const OUTPUT_ITEMS_PATH: &str = "./outputs/items.txt";
pub const OUTPUT_RESULT_PATH: &str = "./outputs/result.txt";
pub const OUTPUT_RESULTS_PATH: &str = "./outputs/results.txt";
pub const ZULIP_OUTPUT_PATH: &str = "./inputs/zulip_messages.json";
pub const ZULIP_CHUNK_SUMMARIES_PATH: &str = "./outputs/zulip_chunk_summaries.txt";
pub const ZULIP_SUMMARY_PATH: &str = "./outputs/zulip_summary.md";
pub const OPENAI_CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";
pub const DEFAULT_ORG: &str = "hoprnet";
pub const DEFAULT_START_DATE: &str = "2026-01-19";
pub const DEFAULT_END_DATE: &str = "2026-02-11";
pub const ZULIP_PAGE_SIZE: usize = 500;
pub const ZULIP_SUMMARY_CHUNK_SIZE: usize = 80;
pub const ZULIP_MESSAGE_CHAR_LIMIT: usize = 700;

pub type AppResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
