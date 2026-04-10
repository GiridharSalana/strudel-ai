use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "Strudel-Ai",
    about = "Generate and play music from a text prompt using Cerebras or Cohere LLMs",
    version,
    long_about = None
)]
pub struct Cli {
    /// Describe the music you want to generate.
    /// Include duration in the prompt ("5 minutes of lo-fi") or use --duration.
    #[arg(value_name = "PROMPT")]
    pub prompt: String,

    /// Target duration of the song. Accepts: "5m", "3min", "300", "2:30".
    /// When set (or detected from the prompt), generates a full multi-section song.
    #[arg(short, long, value_name = "DURATION")]
    pub duration: Option<String>,

    /// LLM provider to use for generation
    #[arg(short, long, value_enum, default_value = "cerebras")]
    pub provider: Provider,

    /// API key (or set CEREBRAS_API_KEY / COHERE_API_KEY env var)
    #[arg(short = 'k', long, value_name = "KEY", env = "STRUDEL_AI_API_KEY")]
    pub api_key: Option<String>,

    /// Model override. Cerebras: llama3.1-8b (default), qwen-3-235b-a22b-instruct-2507.
    /// Cohere: command-a-03-2025 (default)
    #[arg(short, long, value_name = "MODEL")]
    pub model: Option<String>,

    /// Generate pattern but don't play audio
    #[arg(long)]
    pub no_play: bool,

    /// Save output to a file. Use .wav extension for audio, anything else for JSON
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
            Provider::Cohere => "command-a-03-2025",
        }
    }

    pub fn env_key_name(&self) -> &'static str {
        match self {
            Provider::Cerebras => "CEREBRAS_API_KEY",
            Provider::Cohere => "COHERE_API_KEY",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Provider::Cerebras => "Cerebras",
            Provider::Cohere => "Cohere",
        }
    }
}

/// Parse a duration string or detect it from free text.
/// Accepts: "5m", "5min", "5 min", "5 minutes", "300", "300s", "2:30"
pub fn parse_duration_secs(s: &str) -> Option<u32> {
    let s = s.trim().to_lowercase();

    // "2:30" → 150s
    if let Some((mins, secs)) = s.split_once(':') {
        if let (Ok(m), Ok(s)) = (mins.trim().parse::<u32>(), secs.trim().parse::<u32>()) {
            return Some(m * 60 + s);
        }
    }

    // Strip unit suffix to get the number
    let (num_str, unit) = if let Some(pos) = s.find(|c: char| c.is_alphabetic()) {
        (&s[..pos], s[pos..].trim())
    } else {
        (s.as_str(), "s") // bare number → seconds
    };

    let n: f32 = num_str.trim().parse().ok()?;

    if unit.starts_with("h") {
        Some((n * 3600.0) as u32)
    } else if unit.starts_with("m") {
        Some((n * 60.0) as u32)
    } else {
        Some(n as u32)
    }
}

/// Scan free-form prompt text for duration hints like "5 min", "3 minutes", "2 mins".
pub fn extract_duration_from_prompt(prompt: &str) -> Option<u32> {
    let words: Vec<&str> = prompt.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let n: f32 = match word.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(unit) = words.get(i + 1) {
            let unit = unit.to_lowercase();
            if unit.starts_with("min") {
                return Some((n * 60.0) as u32);
            }
            if unit.starts_with("sec") {
                return Some(n as u32);
            }
            if unit.starts_with("hour") || unit.starts_with("hr") {
                return Some((n * 3600.0) as u32);
            }
        }
    }
    None
}
