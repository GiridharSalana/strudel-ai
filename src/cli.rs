use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "Strudel-Ai",
    about = "Generate and play music from a text prompt using Cerebras or Cohere LLMs",
    version,
    long_about = None
)]
pub struct Cli {
    /// Describe the music you want to generate
    #[arg(value_name = "PROMPT")]
    pub prompt: String,

    /// LLM provider to use for generation
    #[arg(short, long, value_enum, default_value = "cerebras")]
    pub provider: Provider,

    /// API key (falls back to CEREBRAS_API_KEY or COHERE_API_KEY env vars)
    #[arg(short = 'k', long, value_name = "KEY", env = "STRUDEL_AI_API_KEY")]
    pub api_key: Option<String>,

    /// Model to use. Cerebras: llama3.1-8b (default), qwen-3-235b-a22b-instruct-2507.
    /// Cohere: command-a-03-2025 (default), command-r-plus
    #[arg(short, long, value_name = "MODEL")]
    pub model: Option<String>,

    /// Generate pattern but don't play audio
    #[arg(long)]
    pub no_play: bool,

    /// Save output to a file. Use .wav extension for audio, any other for pattern JSON
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
