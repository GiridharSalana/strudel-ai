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
use llm::{LlmRequest, build_arrangement, extract_json};
use player::{parse_pattern, play_pattern, play_song, save_pattern_json, save_wav_file};

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!();
        eprintln!("  {} {:#}", "error".red().bold(), e);
        eprintln!();
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let api_key = resolve_api_key(&cli)?;
    let model   = cli.model.clone().unwrap_or_else(|| cli.provider.default_model().to_string());

    let duration_secs: Option<u32> = cli
        .duration
        .as_deref()
        .and_then(parse_duration_secs)
        .or_else(|| extract_duration_from_prompt(&cli.prompt));

    print_header(&cli, &model, duration_secs);

    if let Some(target_secs) = duration_secs {
        run_song_mode(&cli, &api_key, &model, target_secs).await
    } else {
        run_pattern_mode(&cli, &api_key, &model).await
    }
}

// ── Song mode ─────────────────────────────────────────────────────────────────

async fn run_song_mode(cli: &Cli, api_key: &str, model: &str, target_secs: u32) -> Result<()> {
    let total = llm::SONG_SECTIONS.len();
    let mut sections_json: Vec<(String, String)> = Vec::new();

    for (i, spec) in llm::SONG_SECTIONS.iter().enumerate() {
        let pb = spinner(&format!(
            "  composing {:<10} {}/{}",
            spec.name,
            i + 1,
            total
        ));

        let bpm_line = sections_json
            .first()
            .and_then(|(_, j)| serde_json::from_str::<serde_json::Value>(j).ok())
            .and_then(|v| v.get("bpm").and_then(|b| b.as_u64()))
            .map(|b| format!("Use exactly BPM: {b}. Do not change the tempo."))
            .unwrap_or_else(|| "Choose an appropriate BPM for the style (60–160).".into());

        let prompt = format!(
            "Style: {}\nSection: {} ({} bars)\nRole: {}\n{bpm_line}\n\n{}",
            cli.prompt, spec.name, spec.bars, spec.role, llm::FORMAT_RULES
        );

        let raw = call_llm(cli, api_key, model, &prompt).await?;
        let json = extract_json(raw);

        pb.finish_and_clear();
        print_step_done(spec.name, &format!("{} bars", spec.bars));

        sections_json.push((spec.name.to_string(), json));
    }

    // Parse sections
    let mut section_map: HashMap<String, player::Pattern> = HashMap::new();
    let mut bpm = 120.0f32;

    for (name, json) in &sections_json {
        let pattern = parse_pattern(json)
            .with_context(|| format!("Failed to parse section '{name}'"))?;
        if name == "intro" { bpm = pattern.bpm; }
        if cli.print_code {
            let pretty = serde_json::to_string_pretty(
                &serde_json::from_str::<serde_json::Value>(json).unwrap_or_default()
            ).unwrap_or_else(|_| json.clone());
            println!("\n  {} {}\n{}", "▸".dimmed(), name.bold(), pretty.bright_cyan());
        }
        section_map.insert(name.clone(), pattern);
    }

    let arrangement = build_arrangement(bpm, target_secs);
    let total_secs: f32 = arrangement.iter()
        .filter_map(|n| section_map.get(n))
        .map(|p| p.bars as f32 * 4.0 * 60.0 / p.bpm)
        .sum();

    println!();
    print_divider();
    println!(
        "  {}  {}",
        "arrangement".dimmed(),
        arrangement.join(" › ").cyan()
    );
    println!(
        "  {}       {:.0}:{:02.0}",
        "length".dimmed(),
        (total_secs / 60.0).floor(),
        total_secs % 60.0
    );
    print_divider();
    println!();

    if let Some(out) = &cli.output {
        let map: serde_json::Map<String, serde_json::Value> = sections_json
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::from_str(v).unwrap_or_default()))
            .collect();
        save_pattern_json(
            &serde_json::to_string_pretty(&map).unwrap_or_default(),
            std::path::Path::new(out),
        )?;
        println!("  {}  {}", "saved".green().bold(), out.underline());
        println!();
    }

    if !cli.no_play {
        print_playing();
        play_song(&section_map, &arrangement, target_secs)?;
        println!("  {}", "done.".dimmed());
    }

    Ok(())
}

// ── Pattern mode ──────────────────────────────────────────────────────────────

async fn run_pattern_mode(cli: &Cli, api_key: &str, model: &str) -> Result<()> {
    let pb = spinner("  composing...");

    let pattern_json = {
        let req = LlmRequest {
            prompt: cli.prompt.clone(),
            model: model.to_string(),
            api_key: api_key.to_string(),
        };
        let raw = match &cli.provider {
            cli::Provider::Cerebras => llm::cerebras::complete(&req).await?,
            cli::Provider::Cohere   => llm::cohere::complete(&req).await?,
        };
        extract_json(raw)
    };

    pb.finish_and_clear();

    let pattern = parse_pattern(&pattern_json)
        .context("LLM returned invalid JSON — try again or switch model")?;

    print_step_done(
        "pattern",
        &format!("{} bars · {} BPM · {} events", pattern.bars, pattern.bpm as u32, pattern.events.len()),
    );

    if cli.print_code {
        let pretty = serde_json::to_string_pretty(
            &serde_json::from_str::<serde_json::Value>(&pattern_json).unwrap_or_default(),
        )
        .unwrap_or_else(|_| pattern_json.clone());
        println!();
        print_divider();
        println!("{}", pretty.bright_cyan());
        print_divider();
    }

    println!();

    if let Some(out) = &cli.output {
        let p = std::path::Path::new(out);
        if out.ends_with(".wav") {
            save_wav_file(&pattern, p)?;
        } else {
            save_pattern_json(&pattern_json, p)?;
        }
        println!("  {}  {}", "saved".green().bold(), out.underline());
        println!();
    }

    if !cli.no_play {
        print_playing();
        play_pattern(&pattern)?;
        println!("  {}", "done.".dimmed());
    }

    Ok(())
}

// ── Shared LLM call ───────────────────────────────────────────────────────────

async fn call_llm(cli: &Cli, api_key: &str, model: &str, prompt: &str) -> Result<String> {
    let req = LlmRequest {
        prompt: prompt.to_string(),
        model: model.to_string(),
        api_key: api_key.to_string(),
    };
    match &cli.provider {
        cli::Provider::Cerebras => llm::cerebras::complete(&req).await,
        cli::Provider::Cohere   => llm::cohere::complete(&req).await,
    }
}

// ── UI helpers ────────────────────────────────────────────────────────────────

fn print_header(cli: &Cli, model: &str, duration_secs: Option<u32>) {
    println!();
    // Logo bar
    println!(
        "{}{}{}",
        " giribeat ".on_truecolor(99, 102, 241).white().bold(),
        " ".on_truecolor(30, 30, 46),
        format!(" {} ", env!("CARGO_PKG_VERSION")).on_truecolor(30, 30, 46).truecolor(100, 100, 140)
    );
    println!();

    // Info grid — right-align the labels
    let label = |s: &str| format!("{:>10}", s).truecolor(80, 80, 110).to_string();

    println!("  {}  {}", label("prompt"), cli.prompt.white().bold());
    println!("  {}  {} · {}", label("model"), cli.provider.display_name().truecolor(139, 92, 246), model.truecolor(100, 149, 237));

    if let Some(secs) = duration_secs {
        println!(
            "  {}  {}:{:02}  {}",
            label("duration"),
            secs / 60,
            secs % 60,
            "song mode".truecolor(16, 185, 129)
        );
    } else {
        println!("  {}  {}", label("mode"), "loop pattern".truecolor(100, 100, 140));
    }
    println!();
}

fn print_step_done(name: &str, detail: &str) {
    println!(
        "  {}  {:<12}  {}",
        "✓".green().bold(),
        name.white(),
        detail.truecolor(100, 100, 140)
    );
}

fn print_playing() {
    println!(
        "  {}  {}",
        "♪".truecolor(139, 92, 246).bold(),
        "playing — press Ctrl+C to stop".truecolor(100, 100, 140)
    );
    println!();
}

fn print_divider() {
    println!("  {}", "─".repeat(56).truecolor(50, 50, 70));
}

fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.purple} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

fn resolve_api_key(cli: &Cli) -> Result<String> {
    if let Some(k) = &cli.api_key {
        return Ok(k.clone());
    }
    // Try provider-specific env var, then generic fallback
    let env_var = cli.provider.env_key_name();
    std::env::var(env_var)
        .or_else(|_| std::env::var("GIRIBEAT_API_KEY"))
        .with_context(|| {
            format!(
                "No API key found.\n\n  Set one of:\n    export {env_var}=your-key\n    export GIRIBEAT_API_KEY=your-key\n\n  Or pass it directly: giribeat --api-key your-key"
            )
        })
}
