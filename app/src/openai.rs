use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::constants::{AppResult, OPENAI_CHAT_COMPLETIONS_URL};

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

pub async fn create_chat_completion(
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
