use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::{LlmRequest, STRUDEL_SYSTEM_PROMPT};

const BASE_URL: &str = "https://api.cohere.com/v2/chat";

#[derive(Serialize)]
struct Request {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct Response {
    message: AssistantMessage,
}

#[derive(Deserialize)]
struct AssistantMessage {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
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
        max_tokens: 2048,
    };

    let response = client
        .post(BASE_URL)
        .bearer_auth(&req.api_key)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .context("Failed to connect to Cohere API")?;

    let status = response.status();
    let text = response.text().await.context("Failed to read response body")?;

    if !status.is_success() {
        if let Ok(err) = serde_json::from_str::<ErrorResponse>(&text) {
            bail!("Cohere API error {}: {}", status, err.message);
        }
        bail!("Cohere API error {}: {}", status, text);
    }

    let parsed: Response = serde_json::from_str(&text)
        .with_context(|| format!("Failed to parse Cohere response: {text}"))?;

    parsed
        .message
        .content
        .into_iter()
        .find(|b| b.kind == "text")
        .map(|b| b.text)
        .context("Cohere returned no text content")
}
