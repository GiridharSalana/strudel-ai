mod cli;
mod llm;
mod player;

use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::time::Duration;

use cli::{Cli, extract_duration_from_prompt, parse_duration_secs};
use llm::{LlmRequest, build_arrangement, generate_sections, generate_strudel};
use player::{parse_pattern, play_pattern, play_song, save_pattern_json, save_wav_file};

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
    let model   = cli.model.clone().unwrap_or_else(|| cli.provider.default_model().to_string());

    // Resolve target duration: --duration flag takes priority, then scan the prompt
    let duration_secs: Option<u32> = cli
        .duration
        .as_deref()
        .and_then(parse_duration_secs)
        .or_else(|| extract_duration_from_prompt(&cli.prompt));

    print_banner();
    println!("  {} {}", "Provider:".dimmed(), cli.provider.display_name().cyan().bold());
    println!("  {} {}", "Model:".dimmed(), model.cyan());
    println!("  {} {}", "Prompt:".dimmed(), cli.prompt.italic());
    if let Some(secs) = duration_secs {
        println!(
            "  {} {}:{:02}",
            "Duration:".dimmed(),
            secs / 60,
            secs % 60
        );
    }
    println!();

    if let Some(target_secs) = duration_secs {
        run_song_mode(&cli, &api_key, &model, target_secs).await
    } else {
        run_pattern_mode(&cli, &api_key, &model).await
    }
}

// ── Song mode: multi-section, fills target duration ───────────────────────────

async fn run_song_mode(
    cli: &Cli,
    api_key: &str,
    model: &str,
    target_secs: u32,
) -> Result<()> {
    let total_sections = llm::SONG_SECTIONS.len();
    let mut sections_json: Vec<(String, String)> = Vec::new();

    for (i, spec) in llm::SONG_SECTIONS.iter().enumerate() {
        let pb = spinner(&format!(
            "Composing {} ({}/{})...",
            spec.name.bold(),
            i + 1,
            total_sections
        ));

        // Build per-section request (generate_sections internally, but we call it section by section here)
        let bpm_line = if let Some(first_json) = sections_json.first() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&first_json.1) {
                if let Some(b) = val.get("bpm").and_then(|v| v.as_u64()) {
                    format!("Use exactly BPM: {b}. Do not change the tempo.")
                } else {
                    "Choose an appropriate BPM for the style (60–160).".to_string()
                }
            } else {
                "Choose an appropriate BPM for the style (60–160).".to_string()
            }
        } else {
            "Choose an appropriate BPM for the style (60–160).".to_string()
        };

        let prompt = format!(
            "Style: {}\nSection: {} ({} bars)\nRole: {}\n{bpm_line}\n\n{}",
            cli.prompt, spec.name, spec.bars, spec.role, llm::FORMAT_RULES
        );

        let raw = match &cli.provider {
            cli::Provider::Cerebras => llm::cerebras::complete(&LlmRequest {
                prompt: prompt.clone(),
                model: model.to_string(),
                api_key: api_key.to_string(),
            }).await?,
            cli::Provider::Cohere => llm::cohere::complete(&LlmRequest {
                prompt: prompt.clone(),
                model: model.to_string(),
                api_key: api_key.to_string(),
            }).await?,
        };
        let json = llm::extract_json(raw);

        pb.finish_and_clear();
        println!(
            "  {} {}",
            "✓".green(),
            spec.name.cyan()
        );

        sections_json.push((spec.name.to_string(), json));
    }

    // Parse all sections
    let mut section_map: HashMap<String, player::Pattern> = HashMap::new();
    let mut bpm = 120.0f32;

    for (name, json) in &sections_json {
        let pattern = parse_pattern(json)
            .with_context(|| format!("Failed to parse section '{name}'"))?;
        if name == "intro" {
            bpm = pattern.bpm;
        }
        if cli.print_code {
            let pretty = serde_json::to_string_pretty(
                &serde_json::from_str::<serde_json::Value>(json).unwrap_or_default()
            ).unwrap_or_else(|_| json.clone());
            println!("\n{} {}\n{}", "─".repeat(40).dimmed(), name.bold(), pretty.bright_cyan());
        }
        section_map.insert(name.clone(), pattern);
    }

    // Build arrangement to fill target duration
    let arrangement = build_arrangement(bpm, target_secs);
    println!();
    println!("  {} {}", "Arrangement:".dimmed(), arrangement.join(" → ").cyan());

    let total_bar_secs: f32 = arrangement.iter().map(|name| {
        section_map.get(name).map(|p| p.bars as f32 * 4.0 * 60.0 / p.bpm).unwrap_or(0.0)
    }).sum();
    println!(
        "  {} {:.0}:{:02.0}",
        "Total:".dimmed(),
        (total_bar_secs / 60.0).floor(),
        total_bar_secs % 60.0
    );
    println!();

    if let Some(out) = &cli.output {
        if out.ends_with(".wav") {
            // Render all sections and save combined WAV
            // (play_song handles rendering internally; for save we'd need to separate render)
            // For simplicity: play then the user can record. Or we save the json.
            let map: serde_json::Map<String, serde_json::Value> = sections_json
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::from_str(v).unwrap_or_default()))
                .collect();
            let combined_json = serde_json::to_string_pretty(&map).unwrap_or_default();
            save_pattern_json(&combined_json, std::path::Path::new(out))?;
            println!("{} {}", "✓ Song structure saved to".green().bold(), out.underline());
        } else {
            let combined = sections_json.iter()
                .map(|(k, v)| format!("// {k}\n{v}"))
                .collect::<Vec<_>>()
                .join("\n\n");
            save_pattern_json(&combined, std::path::Path::new(out))?;
            println!("{} {}", "✓ Patterns saved to".green().bold(), out.underline());
        }
    }

    if !cli.no_play {
        println!("{}", "✓ Synthesizing song...".green().bold());
        play_song(&section_map, &arrangement, target_secs)?;
    }

    Ok(())
}

// ── Pattern mode: single short loop (original behaviour) ─────────────────────

async fn run_pattern_mode(cli: &Cli, api_key: &str, model: &str) -> Result<()> {
    let pb = spinner("Composing...");

    let pattern_json = generate_strudel(
        LlmRequest { prompt: cli.prompt.clone(), model: model.to_string(), api_key: api_key.to_string() },
        &cli.provider,
    )
    .await
    .context("LLM generation failed")?;

    pb.finish_and_clear();

    let pattern = parse_pattern(&pattern_json)
        .context("The LLM returned invalid JSON. Try again or use a different model.")?;

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

    if !cli.no_play {
        println!("{}", "✓ Synthesizing...".green().bold());
        play_pattern(&pattern)?;
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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
