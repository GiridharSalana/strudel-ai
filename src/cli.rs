use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "giribeat",
    about = "AI-powered music composer for the terminal",
    long_about = "Describe music in plain English. giribeat composes it, synthesizes it, and plays it — entirely in your terminal.",
    version,
    after_help = "Examples:\n  giribeat \"lo-fi hip hop with jazzy piano\"\n  giribeat \"dark techno\" --duration 5m\n  giribeat \"ambient drone\" --output track.wav\n  giribeat \"jazz trio\" --provider cohere --print-code"
)]
pub struct Cli {
    /// What music to generate — describe freely in plain English.
    /// Include a duration to generate a full song ("5 minutes of lo-fi beats").
    #[arg(value_name = "PROMPT")]
    pub prompt: String,

    /// Target song duration. Triggers multi-section song mode.
    /// Formats: 5m · 3min · 2:30 · 300 (seconds)
    #[arg(short, long, value_name = "DURATION")]
    pub duration: Option<String>,

    /// LLM backend to use for composition
    #[arg(short, long, value_enum, default_value = "cerebras")]
    pub provider: Provider,

    /// API key. Falls back to CEREBRAS_API_KEY or COHERE_API_KEY env vars
    #[arg(short = 'k', long, value_name = "KEY", env = "GIRIBEAT_API_KEY")]
    pub api_key: Option<String>,

    /// Override the model. Default depends on provider.
    /// Cerebras: llama3.1-8b, qwen-3-235b-a22b-instruct-2507
    /// Cohere: command-a-03-2025
    #[arg(short, long, value_name = "MODEL")]
    pub model: Option<String>,

    /// Compose but don't play audio
    #[arg(long)]
    pub no_play: bool,

    /// Save output. Use .wav for audio export, any other extension for pattern JSON
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<String>,

    /// Print the generated pattern JSON to stdout
    #[arg(long)]
    pub print_code: bool,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum Provider {
    Cerebras,
    Cohere,
}

impl Provider {
    pub fn default_model(&self) -> &'static str {
        match self {
            Provider::Cerebras => "llama3.1-8b",
            Provider::Cohere   => "command-a-03-2025",
        }
    }

    pub fn env_key_name(&self) -> &'static str {
        match self {
            Provider::Cerebras => "CEREBRAS_API_KEY",
            Provider::Cohere   => "COHERE_API_KEY",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Provider::Cerebras => "Cerebras",
            Provider::Cohere   => "Cohere",
        }
    }
}

/// Parse a duration string into seconds.
/// Accepts: "5m", "5min", "5 min", "5 minutes", "300", "2:30"
pub fn parse_duration_secs(s: &str) -> Option<u32> {
    let s = s.trim().to_lowercase();

    // "2:30" → 150s
    if let Some((mins, secs)) = s.split_once(':') {
        if let (Ok(m), Ok(s)) = (mins.trim().parse::<u32>(), secs.trim().parse::<u32>()) {
            return Some(m * 60 + s);
        }
    }

    let (num_str, unit) = if let Some(pos) = s.find(|c: char| c.is_alphabetic()) {
        (&s[..pos], s[pos..].trim())
    } else {
        (s.as_str(), "s")
    };

    let n: f32 = num_str.trim().parse().ok()?;

    if unit.starts_with('h') {
        Some((n * 3600.0) as u32)
    } else if unit.starts_with('m') {
        Some((n * 60.0) as u32)
    } else {
        Some(n as u32)
    }
}

/// Scan prompt text for natural-language duration hints.
/// Handles: "5 min", "3 minutes", "1min", "2m", "30s", "1.5 hours", "2:30"
pub fn extract_duration_from_prompt(prompt: &str) -> Option<u32> {
    let words: Vec<&str> = prompt.split_whitespace().collect();

    for (i, word) in words.iter().enumerate() {
        // Try "5 min" / "3 minutes" / "1 hour" (number + separate unit)
        if let Ok(n) = word.parse::<f32>() {
            if let Some(unit) = words.get(i + 1) {
                let unit = unit.to_lowercase();
                if unit.starts_with("min") { return Some((n * 60.0) as u32); }
                if unit.starts_with("sec") { return Some(n as u32); }
                if unit.starts_with("hour") || unit.starts_with("hr") { return Some((n * 3600.0) as u32); }
            }
        }

        // Try "1min", "2m", "30s", "1.5hr" (number+unit glued together)
        let w = word.to_lowercase();
        if let Some(pos) = w.find(|c: char| c.is_alphabetic()) {
            if let Ok(n) = w[..pos].parse::<f32>() {
                let unit = &w[pos..];
                if unit.starts_with("min") || unit == "m" { return Some((n * 60.0) as u32); }
                if unit.starts_with("sec") || unit == "s" { return Some(n as u32); }
                if unit.starts_with("hour") || unit.starts_with("hr") || unit == "h" {
                    return Some((n * 3600.0) as u32);
                }
            }
        }

        // Try "2:30" inline
        if let Some((mins, secs)) = word.split_once(':') {
            if let (Ok(m), Ok(s)) = (mins.parse::<u32>(), secs.parse::<u32>()) {
                return Some(m * 60 + s);
            }
        }
    }
    None
}
