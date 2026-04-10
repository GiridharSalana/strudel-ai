use anyhow::{Context, Result};
use colored::Colorize;
use serde::Deserialize;
use std::f32::consts::PI;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const SAMPLE_RATE: u32 = 44100;
const LOOP_COUNT: u32 = 8;

#[derive(Deserialize, Debug, Clone)]
pub struct Pattern {
    pub bpm: f32,
    #[serde(default = "default_bars")]
    pub bars: u32,
    pub events: Vec<MusicEvent>,
}

fn default_bars() -> u32 {
    4
}

#[derive(Deserialize, Debug, Clone)]
pub struct MusicEvent {
    /// Start time in beats (0.0 = bar 1 beat 1)
    pub t: f32,
    /// Duration in beats
    pub dur: f32,
    /// Note name ("C4", "Eb3") or drum ("kick", "snare", "hat", "clap")
    pub note: String,
    /// Waveform for melodic notes: sine, square, sawtooth, triangle
    #[serde(default)]
    pub wave: Option<String>,
    #[serde(default = "default_gain")]
    pub gain: f32,
}

fn default_gain() -> f32 {
    0.5
}

pub fn parse_pattern(json: &str) -> Result<Pattern> {
    serde_json::from_str(json)
        .context("Failed to parse music pattern — LLM may have returned malformed JSON")
}

pub fn save_wav_file(pattern: &Pattern, path: &Path) -> Result<()> {
    let wav = render_wav(pattern);
    std::fs::write(path, &wav)
        .with_context(|| format!("Failed to write WAV to {}", path.display()))
}

pub fn save_pattern_json(json: &str, path: &Path) -> Result<()> {
    std::fs::write(path, json)
        .with_context(|| format!("Failed to write pattern to {}", path.display()))
}

pub fn play_pattern(pattern: &Pattern) -> Result<()> {
    let wav = render_wav(pattern);
    let loop_secs = (pattern.bars as f32 * 4.0 * 60.0) / pattern.bpm;
    let total_secs = loop_secs * LOOP_COUNT as f32;

    println!(
        "  {} {} bars · {} BPM · {} events · {:.1}s × {} loops ({:.0}s total)",
        "♪".cyan().bold(),
        pattern.bars,
        pattern.bpm as u32,
        pattern.events.len(),
        loop_secs,
        LOOP_COUNT,
        total_secs,
    );
    println!("  {} {}", "Stop:".dimmed(), "Ctrl+C".bold());
    println!();

    // Find a working audio player
    let player = detect_player()?;

    for _ in 0..LOOP_COUNT {
        play_wav_bytes(&wav, &player)?;
    }
    Ok(())
}

fn detect_player() -> Result<String> {
    for candidate in &["aplay", "paplay", "ffplay", "mpv", "cvlc"] {
        if Command::new("which")
            .arg(candidate)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(candidate.to_string());
        }
    }
    anyhow::bail!(
        "No audio player found. Install one of: aplay (alsa-utils), paplay (pulseaudio-utils), ffplay, mpv"
    )
}

fn play_wav_bytes(wav: &[u8], player: &str) -> Result<()> {
    // Pipe WAV bytes to the player's stdin. All tested players accept WAV from stdin.
    let args: Vec<&str> = match player {
        "aplay"  => vec!["-q", "-"],
        "paplay" => vec!["-"],
        "ffplay" => vec!["-nodisp", "-autoexit", "-loglevel", "quiet", "-"],
        "mpv"    => vec!["--no-video", "--really-quiet", "-"],
        "cvlc"   => vec!["-q", "--play-and-exit", "-"],
        _        => vec!["-"],
    };

    let mut child = Command::new(player)
        .args(&args)
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to spawn {player}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(wav)
            .with_context(|| format!("Failed to write audio to {player}"))?;
    }

    child
        .wait()
        .with_context(|| format!("Failed to wait for {player}"))?;
    Ok(())
}

// ── WAV rendering ─────────────────────────────────────────────────────────────

fn render_wav(pattern: &Pattern) -> Vec<u8> {
    let mono = generate_mono(pattern);

    // Convert f32 mono → i16 stereo interleaved
    let stereo: Vec<i16> = mono
        .iter()
        .flat_map(|&s| {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            [v, v]
        })
        .collect();

    encode_wav(&stereo)
}

fn generate_mono(pattern: &Pattern) -> Vec<f32> {
    let beat_secs = 60.0 / pattern.bpm;
    let total_beats = pattern.bars as f32 * 4.0;
    // 0.4s tail for release envelopes
    let total_samples = ((total_beats * beat_secs + 0.4) * SAMPLE_RATE as f32) as usize;
    let mut mix = vec![0.0f32; total_samples];

    for event in &pattern.events {
        let start = (event.t * beat_secs * SAMPLE_RATE as f32) as usize;
        let dur_samples = ((event.dur * beat_secs * SAMPLE_RATE as f32) as usize).max(1);
        let samples = synth_event(event, dur_samples);
        for (i, &s) in samples.iter().enumerate() {
            let idx = start + i;
            if idx < mix.len() {
                mix[idx] += s;
            }
        }
    }

    // Normalize peak to 0.85
    let peak = mix.iter().cloned().fold(0.0f32, |a, b| a.abs().max(b.abs()));
    if peak > 0.85 {
        let scale = 0.85 / peak;
        mix.iter_mut().for_each(|s| *s *= scale);
    }

    // 10ms fade in + fade out to avoid clicks at loop boundaries
    let fade = (0.010 * SAMPLE_RATE as f32) as usize;
    for i in 0..fade.min(mix.len()) {
        let t = i as f32 / fade as f32;
        mix[i] *= t;
        let tail = mix.len() - 1 - i;
        mix[tail] *= t;
    }

    mix
}

fn encode_wav(samples: &[i16]) -> Vec<u8> {
    let data_size = (samples.len() * 2) as u32;
    let mut buf = Vec::with_capacity(44 + data_size as usize);

    // RIFF header
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_size).to_le_bytes());
    buf.extend_from_slice(b"WAVE");

    // fmt chunk — 16-byte PCM
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());              // PCM
    buf.extend_from_slice(&2u16.to_le_bytes());              // stereo
    buf.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    buf.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes()); // byte rate
    buf.extend_from_slice(&4u16.to_le_bytes());              // block align
    buf.extend_from_slice(&16u16.to_le_bytes());             // bits per sample

    // data chunk
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}

// ── Event dispatch ────────────────────────────────────────────────────────────

fn synth_event(event: &MusicEvent, dur_samples: usize) -> Vec<f32> {
    match event.note.to_lowercase().as_str() {
        "kick" | "bd" => synth_kick(dur_samples, event.gain),
        "snare" | "sd" => synth_snare(dur_samples, event.gain),
        "hat" | "hh" | "hihat" | "openhat" | "oh" => synth_hat(dur_samples, event.gain),
        "clap" | "cp" => synth_clap(dur_samples, event.gain),
        _ => match note_to_freq(&event.note) {
            Some(freq) => {
                synth_osc(freq, event.wave.as_deref().unwrap_or("sine"), dur_samples, event.gain)
            }
            None => vec![0.0; dur_samples],
        },
    }
}

// ── Frequency table ───────────────────────────────────────────────────────────

fn note_to_freq(note: &str) -> Option<f32> {
    let note = note.trim();
    if note.len() < 2 {
        return None;
    }
    let (name, rest) = if note.len() >= 2
        && (note.as_bytes().get(1) == Some(&b'#') || note.as_bytes().get(1) == Some(&b'b'))
    {
        (&note[..2], &note[2..])
    } else {
        (&note[..1], &note[1..])
    };

    let octave: i32 = rest.trim().parse().ok()?;
    let semitone: i32 = match name.to_uppercase().as_str() {
        "C" => 0,
        "C#" | "DB" => 1,
        "D" => 2,
        "D#" | "EB" => 3,
        "E" => 4,
        "F" => 5,
        "F#" | "GB" => 6,
        "G" => 7,
        "G#" | "AB" => 8,
        "A" => 9,
        "A#" | "BB" => 10,
        "B" => 11,
        _ => return None,
    };
    let midi = (octave + 1) * 12 + semitone; // C4 = MIDI 60
    Some(440.0 * 2.0f32.powf((midi as f32 - 69.0) / 12.0))
}

// ── Oscillator ────────────────────────────────────────────────────────────────

fn synth_osc(freq: f32, wave: &str, dur_samples: usize, gain: f32) -> Vec<f32> {
    let attack = (0.012 * SAMPLE_RATE as f32) as usize;
    let release = (0.07 * SAMPLE_RATE as f32) as usize;
    let mut phase = 0.0f32;
    let inc = freq / SAMPLE_RATE as f32;

    (0..dur_samples)
        .map(|i| {
            let raw = match wave {
                "sine" => (phase * 2.0 * PI).sin(),
                "square" | "sq" => {
                    if phase < 0.5 { 1.0 } else { -1.0 }
                }
                "sawtooth" | "saw" => phase * 2.0 - 1.0,
                "triangle" | "tri" => 4.0 * (phase - (phase + 0.5).floor()).abs() - 1.0,
                _ => (phase * 2.0 * PI).sin(),
            };
            let env = adsr(i, dur_samples, attack, release);
            phase = (phase + inc) % 1.0;
            raw * env * gain
        })
        .collect()
}

// ── Drum synthesis ────────────────────────────────────────────────────────────

fn synth_kick(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = ((0.25 * SAMPLE_RATE as f32) as usize).min(dur_samples);
    let mut phase = 0.0f32;
    (0..dur_samples)
        .map(|i| {
            if i >= cap {
                return 0.0;
            }
            let t = i as f32 / SAMPLE_RATE as f32;
            let freq = 120.0 * (-t * 30.0).exp() + 40.0;
            let env = (-t * 9.0).exp();
            let s = (phase * 2.0 * PI).sin() * env * gain;
            phase = (phase + freq / SAMPLE_RATE as f32) % 1.0;
            s
        })
        .collect()
}

fn synth_snare(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = ((0.13 * SAMPLE_RATE as f32) as usize).min(dur_samples);
    let mut lcg = 0xDEAD_BEEFu32;
    (0..dur_samples)
        .map(|i| {
            if i >= cap {
                return 0.0;
            }
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = (-t * 22.0).exp();
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;
            noise * env * gain
        })
        .collect()
}

fn synth_hat(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = ((0.055 * SAMPLE_RATE as f32) as usize).min(dur_samples);
    let mut lcg = 0xFACE_B00Cu32;
    (0..dur_samples)
        .map(|i| {
            if i >= cap {
                return 0.0;
            }
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = (-t * 55.0).exp();
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;
            noise * env * gain * 0.5
        })
        .collect()
}

fn synth_clap(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = ((0.045 * SAMPLE_RATE as f32) as usize).min(dur_samples);
    let mut lcg = 0xCAFE_BABEu32;
    (0..dur_samples)
        .map(|i| {
            if i >= cap {
                return 0.0;
            }
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = (-t * 35.0).exp();
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;
            noise * env * gain
        })
        .collect()
}

// ── ADSR envelope ─────────────────────────────────────────────────────────────

fn adsr(i: usize, total: usize, attack: usize, release: usize) -> f32 {
    if i < attack {
        i as f32 / attack as f32
    } else if total > release && i >= total - release {
        (total - i) as f32 / release as f32
    } else {
        1.0
    }
}
