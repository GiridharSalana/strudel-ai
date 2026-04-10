mod cli;
mod llm;
mod player;

use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

use cli::Cli;
use llm::{LlmRequest, generate_strudel};
use player::{parse_pattern, play_pattern, save_wav_file, save_pattern_json};

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{} {:#}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let api_key = resolve_api_key(&cli)?;
    let model = cli
        .model
        .clone()
        .unwrap_or_else(|| cli.provider.default_model().to_string());

    print_banner();
    println!("  {} {}", "Provider:".dimmed(), cli.provider.display_name().cyan().bold());
    println!("  {} {}", "Model:".dimmed(), model.cyan());
    println!("  {} {}", "Prompt:".dimmed(), cli.prompt.italic());
    println!();

    let pb = spinner("Composing...");

    let pattern_json = generate_strudel(
        LlmRequest { prompt: cli.prompt.clone(), model, api_key },
        &cli.provider,
    )
    .await
    .context("LLM generation failed")?;

    pb.finish_and_clear();

    let pattern = parse_pattern(&pattern_json).context(
        "The LLM returned invalid JSON. Try again or use a different model."
    )?;

    println!(
        "{} {} bars · {} BPM · {} events",
        "✓ Pattern ready:".green().bold(),
        pattern.bars,
        pattern.bpm as u32,
        pattern.events.len()
    );

    if cli.print_code {
        let pretty = serde_json::to_string_pretty(
            &serde_json::from_str::<serde_json::Value>(&pattern_json).unwrap_or_default(),
        )
        .unwrap_or_else(|_| pattern_json.clone());
        println!("\n{}", "─".repeat(60).dimmed());
        println!("{}", pretty.bright_cyan());
        println!("{}\n", "─".repeat(60).dimmed());
    }

    if let Some(out) = &cli.output {
        let p = std::path::Path::new(out);
        if out.ends_with(".wav") {
            save_wav_file(&pattern, p)?;
        } else {
            save_pattern_json(&pattern_json, p)?;
        }
        println!("{} {}", "✓ Saved to".green().bold(), out.underline());
    }

    if cli.no_play {
        return Ok(());
    }

    println!("{}", "✓ Synthesizing...".green().bold());
    play_pattern(&pattern)?;

    println!("{}", "Done.".dimmed());
    Ok(())
}

fn resolve_api_key(cli: &Cli) -> Result<String> {
    if let Some(k) = &cli.api_key {
        return Ok(k.clone());
    }
    let env_var = cli.provider.env_key_name();
    std::env::var(env_var).with_context(|| {
        format!(
            "No API key found. Provide --api-key or set the {} environment variable.",
            env_var
        )
    })
}

fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

fn print_banner() {
    println!();
    println!("{}", " ♪ strudel-ai ".on_truecolor(124, 58, 237).white().bold());
    println!();
}
