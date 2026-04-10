use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::{LlmRequest, STRUDEL_SYSTEM_PROMPT};

const BASE_URL: &str = "https://api.cerebras.ai/v1/chat/completions";

#[derive(Serialize)]
struct Request {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    max_completion_tokens: u32,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct Response {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

pub async fn complete(req: &LlmRequest) -> Result<String> {
    let client = reqwest::Client::new();

    let body = Request {
        model: req.model.clone(),
        messages: vec![
            Message {
                role: "system".into(),
                content: STRUDEL_SYSTEM_PROMPT.into(),
            },
            Message {
                role: "user".into(),
                content: req.prompt.clone(),
            },
        ],
        temperature: 0.8,
        max_completion_tokens: 2048,
    };

    let response = client
        .post(BASE_URL)
        .bearer_auth(&req.api_key)
        .json(&body)
        .send()
        .await
        .context("Failed to connect to Cerebras API")?;

    let status = response.status();
    let text = response.text().await.context("Failed to read response body")?;

    if !status.is_success() {
        if let Ok(err) = serde_json::from_str::<ErrorResponse>(&text) {
            bail!("Cerebras API error {}: {}", status, err.error.message);
        }
        bail!("Cerebras API error {}: {}", status, text);
    }

    let parsed: Response = serde_json::from_str(&text)
        .with_context(|| format!("Failed to parse Cerebras response: {text}"))?;

    parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .context("Cerebras returned no choices")
}
