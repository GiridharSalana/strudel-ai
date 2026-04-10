use anyhow::{Context, Result};
use colored::Colorize;
use fundsp::prelude::*;
use serde::Deserialize;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub const SAMPLE_RATE: u32 = 44100;

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug, Clone)]
pub struct Pattern {
    pub bpm: f32,
    #[serde(default = "default_bars")]
    pub bars: u32,
    /// Overall reverb wetness. 0.0=dry  0.2=room  0.5=hall  0.8=cathedral
    #[serde(default)]
    pub reverb: Option<f32>,
    pub events: Vec<MusicEvent>,
}

fn default_bars() -> u32 { 4 }

#[derive(Deserialize, Debug, Clone)]
pub struct MusicEvent {
    pub t: f32,
    pub dur: f32,
    pub note: String,
    /// Oscillator shape for melodic notes.
    #[serde(default)]
    pub wave: Option<String>,
    #[serde(default = "default_gain")]
    pub gain: f32,
    /// Stereo position: -1.0 hard-left · 0.0 center · 1.0 hard-right
    #[serde(default)]
    pub pan: Option<f32>,
    /// ADSR attack time in seconds (melodic). 0.001=pluck  0.02=normal  0.3=pad
    #[serde(default)]
    pub attack: Option<f32>,
    /// ADSR release time in seconds (melodic). 0.05=staccato  0.15=normal  0.8=sustained
    #[serde(default)]
    pub release: Option<f32>,
}

fn default_gain() -> f32 { 0.5 }

// ── Parse ─────────────────────────────────────────────────────────────────────

pub fn parse_pattern(json: &str) -> Result<Pattern> {
    let val: serde_json::Value = serde_json::from_str(json)
        .context("Failed to parse music pattern — LLM may have returned malformed JSON")?;
    let clean = serde_json::to_string(&val).context("Failed to re-serialize pattern")?;
    serde_json::from_str(&clean).context("Failed to deserialize pattern from cleaned JSON")
}

// ── Save ──────────────────────────────────────────────────────────────────────

pub fn save_wav_file(pattern: &Pattern, path: &Path) -> Result<()> {
    let wav = render_wav(pattern);
    std::fs::write(path, &wav).with_context(|| format!("Failed to write WAV to {}", path.display()))
}

pub fn save_pattern_json(json: &str, path: &Path) -> Result<()> {
    std::fs::write(path, json)
        .with_context(|| format!("Failed to write pattern to {}", path.display()))
}

/// Render a pattern to stereo-interleaved f32 samples for song mode.
pub fn render_section(pattern: &Pattern) -> Vec<f32> {
    generate_stereo(pattern)
}

// ── Play ──────────────────────────────────────────────────────────────────────

pub fn play_pattern(pattern: &Pattern) -> Result<()> {
    let wav = render_wav(pattern);
    let duration_secs = (pattern.bars as f32 * 4.0 * 60.0) / pattern.bpm;
    println!(
        "  {:<12}  {} bars · {} BPM · {:.1}s · {} events",
        "synthesizing".truecolor(100, 100, 140),
        pattern.bars, pattern.bpm as u32, duration_secs, pattern.events.len(),
    );
    println!();
    let player = detect_player()?;
    play_wav_bytes(&wav, &player)
}

/// Concatenate pre-rendered stereo-interleaved sections with crossfades and play.
pub fn play_song(sections: &[(String, Vec<f32>)], _target_secs: u32) -> Result<()> {
    // ×2 because buffers are stereo-interleaved
    let xfade = (0.040 * SAMPLE_RATE as f32) as usize * 2;
    let mut timeline: Vec<f32> = Vec::new();

    for (_, buf) in sections {
        if timeline.is_empty() {
            timeline.extend_from_slice(buf);
        } else {
            // Keep overlap even so we never split an L/R pair
            let overlap = (std::cmp::min(xfade, std::cmp::min(buf.len(), timeline.len()))) & !1;
            let start = timeline.len() - overlap;
            for i in 0..overlap {
                let t = i as f32 / overlap as f32;
                timeline[start + i] = timeline[start + i] * (1.0 - t) + buf[i] * t;
            }
            timeline.extend_from_slice(&buf[overlap..]);
        }
    }

    // Final normalize across both channels
    let peak = timeline.iter().cloned().fold(0.0f32, |a, b| a.abs().max(b.abs()));
    if peak > 0.85 {
        let scale = 0.85 / peak;
        timeline.iter_mut().for_each(|s| *s *= scale);
    }

    let i16_samples: Vec<i16> = timeline
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();
    let wav = encode_wav(&i16_samples);

    let actual_secs = (timeline.len() / 2) as f32 / SAMPLE_RATE as f32;
    println!(
        "  {:<12}  {:.0}:{:02.0} · {} sections",
        "synthesizing".truecolor(100, 100, 140),
        (actual_secs / 60.0).floor(), actual_secs % 60.0, sections.len(),
    );
    println!();

    let player = detect_player()?;
    play_wav_bytes(&wav, &player)
}

fn detect_player() -> Result<String> {
    for p in &["aplay", "paplay", "ffplay", "mpv", "cvlc"] {
        if Command::new("which").arg(p).output().map(|o| o.status.success()).unwrap_or(false) {
            return Ok(p.to_string());
        }
    }
    anyhow::bail!("No audio player found. Install one of: aplay (alsa-utils), paplay, ffplay, mpv")
}

fn play_wav_bytes(wav: &[u8], player: &str) -> Result<()> {
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
        stdin.write_all(wav).with_context(|| format!("Failed to pipe audio to {player}"))?;
    }
    child.wait().with_context(|| format!("Failed to wait for {player}"))?;
    Ok(())
}

// ── WAV rendering ─────────────────────────────────────────────────────────────

fn render_wav(pattern: &Pattern) -> Vec<u8> {
    let stereo = generate_stereo(pattern);
    let i16_samples: Vec<i16> = stereo
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();
    encode_wav(&i16_samples)
}

/// Core audio engine. All musical parameters are LLM-driven:
///   pattern.reverb     — overall wetness
///   event.pan          — stereo position
///   event.attack       — ADSR attack
///   event.release      — ADSR release
///   event.gain         — amplitude
///   event.wave         — oscillator shape
///
/// Returns stereo-interleaved f32 samples [L0, R0, L1, R1, …].
fn generate_stereo(pattern: &Pattern) -> Vec<f32> {
    let beat_secs    = 60.0 / pattern.bpm;
    let total_beats  = pattern.bars as f32 * 4.0;
    let total_samples = ((total_beats * beat_secs + 0.8) * SAMPLE_RATE as f32) as usize;

    // Separate melodic and drum buses, each in stereo
    let mut mel_l = vec![0.0f32; total_samples];
    let mut mel_r = vec![0.0f32; total_samples];
    let mut drm_l = vec![0.0f32; total_samples];
    let mut drm_r = vec![0.0f32; total_samples];

    for event in &pattern.events {
        let start = (event.t * beat_secs * SAMPLE_RATE as f32) as usize;
        let dur_samples = std::cmp::max(
            (event.dur * beat_secs * SAMPLE_RATE as f32) as usize, 1,
        );
        let drum = is_drum_note(&event.note);
        let samples = synth_event(event, dur_samples);

        // Equal-power panning: angle ∈ [0, π/2]
        let pan   = event.pan.unwrap_or(0.0).clamp(-1.0, 1.0);
        let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        let l_gain = angle.cos();
        let r_gain = angle.sin();

        let (tl, tr) = if drum { (&mut drm_l, &mut drm_r) } else { (&mut mel_l, &mut mel_r) };
        for (i, &s) in samples.iter().enumerate() {
            let idx = start + i;
            if idx < total_samples {
                tl[idx] += s * l_gain;
                tr[idx] += s * r_gain;
            }
        }
    }

    // Reverb only on melodic bus; wetness controlled by LLM
    let wet   = pattern.reverb.unwrap_or(0.22).clamp(0.0, 1.0);
    let mel_l = apply_reverb(&mel_l, wet);
    let mel_r = apply_reverb(&mel_r, wet);

    // Mix: melodic (reverbed) + drums (dry)
    let mut left:  Vec<f32> = mel_l.iter().zip(drm_l.iter()).map(|(&m, &d)| m * 0.85 + d).collect();
    let mut right: Vec<f32> = mel_r.iter().zip(drm_r.iter()).map(|(&m, &d)| m * 0.85 + d).collect();

    // Normalize across both channels
    let peak = left.iter().chain(right.iter()).cloned().fold(0.0f32, |a, b| a.abs().max(b.abs()));
    if peak > 0.85 {
        let scale = 0.85 / peak;
        left.iter_mut().for_each(|s| *s *= scale);
        right.iter_mut().for_each(|s| *s *= scale);
    }

    // 10 ms fade-in/out to prevent clicks at section boundaries
    let fade = std::cmp::min((0.010 * SAMPLE_RATE as f32) as usize, left.len() / 4);
    for i in 0..fade {
        let t = i as f32 / fade as f32;
        left[i]  *= t;  right[i]  *= t;
        let li = left.len() - 1 - i;
        left[li] *= t;
        let ri = right.len() - 1 - i;
        right[ri] *= t;
    }

    // Interleave L/R → [L0, R0, L1, R1, …]
    left.iter().zip(right.iter()).flat_map(|(&l, &r)| [l, r]).collect()
}

fn is_drum_note(note: &str) -> bool {
    matches!(
        note.to_lowercase().as_str(),
        "kick" | "bd" | "snare" | "sd" | "hat" | "hh" | "hihat" | "openhat" | "oh" | "clap" | "cp"
    )
}

// ── Schroeder reverb ──────────────────────────────────────────────────────────

struct CombFilter {
    buf: Vec<f32>, idx: usize,
    feedback: f32, damp1: f32, damp2: f32, filterstore: f32,
}

impl CombFilter {
    fn new(delay_samples: usize, feedback: f32, damp: f32) -> Self {
        Self { buf: vec![0.0; delay_samples], idx: 0, feedback, damp1: damp, damp2: 1.0 - damp, filterstore: 0.0 }
    }
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buf[self.idx];
        self.filterstore = output * self.damp2 + self.filterstore * self.damp1;
        self.buf[self.idx] = input + self.filterstore * self.feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        output
    }
}

struct AllPassFilter { buf: Vec<f32>, idx: usize, feedback: f32 }

impl AllPassFilter {
    fn new(delay_samples: usize) -> Self {
        Self { buf: vec![0.0; delay_samples], idx: 0, feedback: 0.5 }
    }
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buf[self.idx];
        let output = -input + buffered;
        self.buf[self.idx] = input + buffered * self.feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        output
    }
}

/// Schroeder reverb. `wet` 0.0–1.0 is LLM-controlled.
fn apply_reverb(input: &[f32], wet: f32) -> Vec<f32> {
    let sr = SAMPLE_RATE as f32;
    let mut combs = [
        CombFilter::new((sr * 0.0297) as usize, 0.84, 0.2),
        CombFilter::new((sr * 0.0371) as usize, 0.84, 0.2),
        CombFilter::new((sr * 0.0411) as usize, 0.84, 0.2),
        CombFilter::new((sr * 0.0437) as usize, 0.84, 0.2),
    ];
    let mut allpasses = [
        AllPassFilter::new((sr * 0.005)  as usize),
        AllPassFilter::new((sr * 0.0017) as usize),
    ];
    let dry = 1.0 - wet;
    input.iter().map(|&s| {
        let verb: f32 = combs.iter_mut().map(|c| c.process(s)).sum::<f32>() * 0.25;
        let verb = allpasses[0].process(verb);
        let verb = allpasses[1].process(verb);
        s * dry + verb * wet
    }).collect()
}

// ── One-pole low-pass ──────────────────────────────────────────────────────────

fn lowpass(samples: &mut [f32], cutoff_hz: f32) {
    let dt    = 1.0 / SAMPLE_RATE as f32;
    let rc    = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
    let alpha = dt / (rc + dt);
    let mut state = 0.0f32;
    for s in samples.iter_mut() {
        state += alpha * (*s - state);
        *s = state;
    }
}

// ── Event dispatch ────────────────────────────────────────────────────────────

fn synth_event(event: &MusicEvent, dur_samples: usize) -> Vec<f32> {
    match event.note.to_lowercase().as_str() {
        "kick" | "bd"                              => synth_kick(dur_samples, event.gain),
        "snare" | "sd"                             => synth_snare(dur_samples, event.gain),
        "hat" | "hh" | "hihat" | "openhat" | "oh" => synth_hat(dur_samples, event.gain),
        "clap" | "cp"                              => synth_clap(dur_samples, event.gain),
        _ => match note_to_freq(&event.note) {
            Some(freq) => {
                let wave = event.wave.as_deref().unwrap_or("sine");
                // LLM-specified ADSR; fall back to adaptive defaults
                let attack_secs  = event.attack.unwrap_or(0.020).clamp(0.001, 2.0);
                let release_secs = event.release.unwrap_or_else(|| {
                    if dur_samples > (SAMPLE_RATE as f32 * 0.4) as usize { 0.30 } else { 0.08 }
                }).clamp(0.001, 4.0);
                synth_osc(freq, wave, dur_samples, event.gain, attack_secs, release_secs)
            }
            None => vec![0.0; dur_samples],
        },
    }
}

// ── Frequency table ───────────────────────────────────────────────────────────

fn note_to_freq(note: &str) -> Option<f32> {
    let note = note.trim();
    if note.len() < 2 { return None; }
    let (name, rest) = if note.len() >= 2
        && (note.as_bytes().get(1) == Some(&b'#') || note.as_bytes().get(1) == Some(&b'b'))
    {
        (&note[..2], &note[2..])
    } else {
        (&note[..1], &note[1..])
    };
    let octave: i32 = rest.trim().parse().ok()?;
    let semitone: i32 = match name.to_uppercase().as_str() {
        "C"         => 0,  "C#" | "DB" => 1,  "D"         => 2,  "D#" | "EB" => 3,
        "E"         => 4,  "F"         => 5,  "F#" | "GB" => 6,  "G"         => 7,
        "G#" | "AB" => 8,  "A"         => 9,  "A#" | "BB" => 10, "B"         => 11,
        _ => return None,
    };
    let midi = (octave + 1) * 12 + semitone;
    Some(440.0 * 2.0f32.powf((midi as f32 - 69.0) / 12.0))
}

// ── Melodic oscillator ─────────────────────────────────────────────────────────

macro_rules! render_fundsp {
    ($osc:expr, $dur:expr, $attack:expr, $release:expr, $gain:expr) => {{
        let mut node = $osc;
        node.set_sample_rate(SAMPLE_RATE as f64);
        node.reset();
        (0..$dur)
            .map(|i| node.get_mono() * adsr_env(i, $dur, $attack, $release) * $gain)
            .collect::<Vec<f32>>()
    }};
}

/// Two-voice detuned chorus oscillator.
/// attack_secs / release_secs are fully LLM-controlled.
fn synth_osc(
    freq: f32, wave: &str, dur_samples: usize,
    gain: f32, attack_secs: f32, release_secs: f32,
) -> Vec<f32> {
    let attack  = std::cmp::min((attack_secs  * SAMPLE_RATE as f32) as usize, dur_samples / 3);
    let release = std::cmp::min((release_secs * SAMPLE_RATE as f32) as usize, dur_samples / 3);

    macro_rules! voice {
        ($f:expr, $g:expr) => {
            match wave {
                "sine"             => render_fundsp!(sine_hz::<f32>($f), dur_samples, attack, release, $g),
                "square" | "sq"    => render_fundsp!(square_hz($f),      dur_samples, attack, release, $g),
                "sawtooth" | "saw" => render_fundsp!(saw_hz($f),         dur_samples, attack, release, $g),
                "triangle" | "tri" => render_fundsp!(triangle_hz($f),    dur_samples, attack, release, $g),
                _                  => render_fundsp!(sine_hz::<f32>($f), dur_samples, attack, release, $g),
            }
        };
    }

    // Voice 1: original  |  Voice 2: +6 cents, 11 ms delayed → classic 2-osc chorus
    let v1    = voice!(freq, gain);
    let f2    = freq * 2.0f32.powf(6.0 / 1200.0);
    let v2    = voice!(f2, gain * 0.55);
    let delay = (0.011 * SAMPLE_RATE as f32) as usize;

    let mut out = vec![0.0f32; dur_samples];
    for i in 0..dur_samples {
        out[i] = v1[i];
        if i >= delay { out[i] += v2[i - delay]; }
    }

    if matches!(wave, "square" | "sq" | "sawtooth" | "saw") {
        lowpass(&mut out, (freq * 4.0).min(7000.0));
    }
    out
}

// ── Drums ─────────────────────────────────────────────────────────────────────

fn synth_kick(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = std::cmp::min((0.28 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut phase = 0.0f32;
    let mut lcg = 0xDEAD_BEEFu32;
    (0..dur_samples).map(|i| {
        if i >= cap { return 0.0; }
        let t = i as f32 / SAMPLE_RATE as f32;
        let freq = 150.0 * (-t * 28.0).exp() + 45.0;
        let tone = (phase * 2.0 * std::f32::consts::PI).sin() * (-t * 8.5).exp();
        phase = (phase + freq / SAMPLE_RATE as f32) % 1.0;
        lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;
        let click = noise * (-t * 200.0).exp() * 0.35;
        (tone + click) * gain
    }).collect()
}

fn synth_snare(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = std::cmp::min((0.18 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut phase = 0.0f32;
    let mut lcg = 0xDEAD_BEEFu32;
    (0..dur_samples).map(|i| {
        if i >= cap { return 0.0; }
        let t = i as f32 / SAMPLE_RATE as f32;
        let tone = (phase * 2.0 * std::f32::consts::PI).sin() * (-t * 35.0).exp() * 0.4;
        phase = (phase + 200.0 / SAMPLE_RATE as f32) % 1.0;
        lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;
        (tone + noise * (-t * 18.0).exp() * 0.7) * gain
    }).collect()
}

fn synth_hat(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = std::cmp::min((0.06 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut lcg = 0xFACE_B00Cu32;
    let mut prev = 0.0f32;
    (0..dur_samples).map(|i| {
        if i >= cap { return 0.0; }
        let t = i as f32 / SAMPLE_RATE as f32;
        lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let raw = (lcg >> 16) as f32 / 32_768.0 - 1.0;
        prev += 0.05 * (raw - prev);
        (raw - prev) * (-t * 60.0).exp() * gain * 0.6
    }).collect()
}

fn synth_clap(dur_samples: usize, gain: f32) -> Vec<f32> {
    let mut lcg = 0xCAFE_BABEu32;
    let burst = (0.008 * SAMPLE_RATE as f32) as usize;
    let gap   = (0.006 * SAMPLE_RATE as f32) as usize;
    (0..dur_samples).map(|i| {
        lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;
        let b = |start: usize| -> f32 {
            if i >= start && i < start + burst {
                let t = (i - start) as f32 / SAMPLE_RATE as f32;
                (-t * 80.0).exp()
            } else { 0.0 }
        };
        noise * (b(0) + b(gap) * 0.7 + b(gap * 2) * 0.5) * gain
    }).collect()
}

// ── ADSR envelope helper ──────────────────────────────────────────────────────

fn adsr_env(i: usize, total: usize, attack: usize, release: usize) -> f32 {
    if i < attack {
        i as f32 / attack as f32
    } else if total > release && i >= total - release {
        (total - i) as f32 / release as f32
    } else {
        1.0
    }
}

// ── WAV encoder ───────────────────────────────────────────────────────────────

fn encode_wav(samples: &[i16]) -> Vec<u8> {
    let data_size = (samples.len() * 2) as u32;
    let mut buf = Vec::with_capacity(44 + data_size as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_size).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());           // PCM
    buf.extend_from_slice(&2u16.to_le_bytes());           // stereo
    buf.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    buf.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes()); // byte rate
    buf.extend_from_slice(&4u16.to_le_bytes());           // block align
    buf.extend_from_slice(&16u16.to_le_bytes());          // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples { buf.extend_from_slice(&s.to_le_bytes()); }
    buf
}
