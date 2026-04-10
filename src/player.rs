use anyhow::{Context, Result};
use colored::Colorize;
use fundsp::prelude::*;
use serde::Deserialize;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub const SAMPLE_RATE: u32 = 44100;
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
    pub t: f32,
    pub dur: f32,
    pub note: String,
    #[serde(default)]
    pub wave: Option<String>,
    #[serde(default = "default_gain")]
    pub gain: f32,
}

fn default_gain() -> f32 {
    0.5
}

pub fn parse_pattern(json: &str) -> Result<Pattern> {
    // Parse to Value first — this tolerates duplicate keys (keeps last value).
    // Then re-serialize to a clean string before deserializing into Pattern.
    let val: serde_json::Value = serde_json::from_str(json)
        .context("Failed to parse music pattern — LLM may have returned malformed JSON")?;
    let clean = serde_json::to_string(&val).context("Failed to re-serialize pattern")?;
    serde_json::from_str(&clean)
        .context("Failed to deserialize pattern from cleaned JSON")
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

/// Render a pattern to a mono f32 sample buffer (used by song mode).
pub fn render_section(pattern: &Pattern) -> Vec<f32> {
    generate_mono(pattern)
}

pub fn play_pattern(pattern: &Pattern) -> Result<()> {
    let wav = render_wav(pattern);
    let loop_secs = (pattern.bars as f32 * 4.0 * 60.0) / pattern.bpm;

    println!(
        "  {:<12}  {} bars · {} BPM · {} events",
        "synthesizing".truecolor(100, 100, 140),
        pattern.bars,
        pattern.bpm as u32,
        pattern.events.len(),
    );
    println!(
        "  {:<12}  {:.1}s × {} loops",
        "",
        loop_secs,
        LOOP_COUNT,
    );
    println!();

    let player = detect_player()?;
    for _ in 0..LOOP_COUNT {
        play_wav_bytes(&wav, &player)?;
    }
    Ok(())
}

/// Play a full multi-section song. Each section is a pre-rendered mono buffer.
/// Sections are concatenated with short crossfades and played as one stream.
pub fn play_song(sections: &[(String, Vec<f32>)], _target_secs: u32) -> Result<()> {
    let xfade = (0.040 * SAMPLE_RATE as f32) as usize; // 40ms crossfade
    let mut timeline: Vec<f32> = Vec::new();

    for (_, buf) in sections {
        if timeline.is_empty() {
            timeline.extend_from_slice(buf);
        } else {
            let overlap = std::cmp::min(xfade, std::cmp::min(buf.len(), timeline.len()));
            let start   = timeline.len() - overlap;
            for i in 0..overlap {
                let t = i as f32 / overlap as f32;
                timeline[start + i] = timeline[start + i] * (1.0 - t) + buf[i] * t;
            }
            timeline.extend_from_slice(&buf[overlap..]);
        }
    }

    // Final normalize
    let peak = timeline.iter().cloned().fold(0.0f32, |a, b| a.abs().max(b.abs()));
    if peak > 0.85 {
        let scale = 0.85 / peak;
        timeline.iter_mut().for_each(|s| *s *= scale);
    }

    let stereo: Vec<i16> = timeline
        .iter()
        .flat_map(|&s| {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            [v, v]
        })
        .collect();
    let wav = encode_wav(&stereo);

    let actual_secs = timeline.len() as f32 / SAMPLE_RATE as f32;
    println!(
        "  {:<12}  {:.0}:{:02.0} · {} sections",
        "synthesizing".truecolor(100, 100, 140),
        (actual_secs / 60.0).floor(),
        actual_secs % 60.0,
        sections.len(),
    );
    println!();

    let player = detect_player()?;
    play_wav_bytes(&wav, &player)
}

fn detect_player() -> Result<String> {
    for p in &["aplay", "paplay", "ffplay", "mpv", "cvlc"] {
        if Command::new("which")
            .arg(p)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(p.to_string());
        }
    }
    anyhow::bail!(
        "No audio player found. Install one of: aplay (alsa-utils), paplay, ffplay, mpv"
    )
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
    let mono = generate_mono(pattern);
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
    let total_samples = ((total_beats * beat_secs + 0.8) * SAMPLE_RATE as f32) as usize;

    let mut melodic_mix = vec![0.0f32; total_samples];
    let mut drum_mix    = vec![0.0f32; total_samples];

    for event in &pattern.events {
        let start = (event.t * beat_secs * SAMPLE_RATE as f32) as usize;
        let dur_samples = std::cmp::max((event.dur * beat_secs * SAMPLE_RATE as f32) as usize, 1);
        let is_drum = matches!(
            event.note.to_lowercase().as_str(),
            "kick" | "bd" | "snare" | "sd" | "hat" | "hh" | "hihat" | "openhat" | "oh" | "clap" | "cp"
        );

        let samples = synth_event(event, dur_samples);
        let target = if is_drum { &mut drum_mix } else { &mut melodic_mix };

        for (i, &s) in samples.iter().enumerate() {
            let idx = start + i;
            if idx < target.len() {
                target[idx] += s;
            }
        }
    }

    // Apply reverb to melodic layer only
    let melodic_wet = apply_reverb(&melodic_mix);

    // Mix drums (dry) + melodic (wet)
    let mut mix: Vec<f32> = melodic_wet
        .iter()
        .zip(drum_mix.iter())
        .map(|(&m, &d)| m * 0.85 + d)
        .collect();

    // Normalize
    let peak = mix.iter().cloned().fold(0.0f32, |a, b| a.abs().max(b.abs()));
    if peak > 0.85 {
        let scale = 0.85 / peak;
        mix.iter_mut().for_each(|s| *s *= scale);
    }

    // 10ms fade in/out to prevent loop clicks
    let fade = (0.010 * SAMPLE_RATE as f32) as usize;
    for i in 0..std::cmp::min(fade, mix.len()) {
        let t = i as f32 / fade as f32;
        mix[i] *= t;
        let tail = mix.len() - 1 - i;
        mix[tail] *= t;
    }

    mix
}

// ── Schroeder reverb ──────────────────────────────────────────────────────────
//
// 4 parallel comb filters → 2 series all-pass filters.
// Classic Schroeder/Freeverb structure.

struct CombFilter {
    buf: Vec<f32>,
    idx: usize,
    feedback: f32,
    damp1: f32,
    damp2: f32,
    filterstore: f32,
}

impl CombFilter {
    fn new(delay_samples: usize, feedback: f32, damp: f32) -> Self {
        Self {
            buf: vec![0.0; delay_samples],
            idx: 0,
            feedback,
            damp1: damp,
            damp2: 1.0 - damp,
            filterstore: 0.0,
        }
    }
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buf[self.idx];
        self.filterstore = output * self.damp2 + self.filterstore * self.damp1;
        self.buf[self.idx] = input + self.filterstore * self.feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        output
    }
}

struct AllPassFilter {
    buf: Vec<f32>,
    idx: usize,
    feedback: f32,
}

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

fn apply_reverb(input: &[f32]) -> Vec<f32> {
    let sr = SAMPLE_RATE as f32;
    // Comb filter delay times (prime-ish lengths, tuned for 44100 Hz)
    let mut combs = [
        CombFilter::new((sr * 0.0297) as usize, 0.84, 0.2),
        CombFilter::new((sr * 0.0371) as usize, 0.84, 0.2),
        CombFilter::new((sr * 0.0411) as usize, 0.84, 0.2),
        CombFilter::new((sr * 0.0437) as usize, 0.84, 0.2),
    ];
    let mut allpasses = [
        AllPassFilter::new((sr * 0.005) as usize),
        AllPassFilter::new((sr * 0.0017) as usize),
    ];

    let wet  = 0.28;
    let dry  = 1.0 - wet;

    input
        .iter()
        .map(|&s| {
            let verb: f32 = combs.iter_mut().map(|c| c.process(s)).sum::<f32>() * 0.25;
            let verb = allpasses[0].process(verb);
            let verb = allpasses[1].process(verb);
            s * dry + verb * wet
        })
        .collect()
}

// ── One-pole low-pass filter (for bass warmth) ────────────────────────────────

fn lowpass(samples: &mut [f32], cutoff_hz: f32) {
    let dt = 1.0 / SAMPLE_RATE as f32;
    let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
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
        "kick" | "bd"                                => synth_kick(dur_samples, event.gain),
        "snare" | "sd"                               => synth_snare(dur_samples, event.gain),
        "hat" | "hh" | "hihat" | "openhat" | "oh"   => synth_hat(dur_samples, event.gain),
        "clap" | "cp"                                => synth_clap(dur_samples, event.gain),
        _ => match note_to_freq(&event.note) {
            Some(freq) => synth_osc(freq, event.wave.as_deref().unwrap_or("sine"), dur_samples, event.gain),
            None       => vec![0.0; dur_samples],
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
        "C"         => 0,
        "C#" | "DB" => 1,
        "D"         => 2,
        "D#" | "EB" => 3,
        "E"         => 4,
        "F"         => 5,
        "F#" | "GB" => 6,
        "G"         => 7,
        "G#" | "AB" => 8,
        "A"         => 9,
        "A#" | "BB" => 10,
        "B"         => 11,
        _           => return None,
    };
    let midi = (octave + 1) * 12 + semitone;
    Some(440.0 * 2.0f32.powf((midi as f32 - 69.0) / 12.0))
}

// ── Melodic oscillator (fundsp anti-aliased) ──────────────────────────────────
//
// fundsp provides PolyBLEP band-limited square and sawtooth oscillators which
// eliminate the harsh aliasing of naive waveforms. Sine and triangle are clean
// by nature but also improved here.

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

fn synth_osc(freq: f32, wave: &str, dur_samples: usize, gain: f32) -> Vec<f32> {
    // Adaptive ADSR: longer release for sustained/pad notes
    let attack  = std::cmp::min((0.020 * SAMPLE_RATE as f32) as usize, dur_samples / 5);
    let is_pad  = dur_samples > (SAMPLE_RATE as f32 * 0.4) as usize;
    let release = if is_pad {
        std::cmp::min((0.30 * SAMPLE_RATE as f32) as usize, dur_samples / 3)
    } else {
        std::cmp::min((0.08 * SAMPLE_RATE as f32) as usize, dur_samples / 3)
    };

    // Local helper macro to render one oscillator voice
    macro_rules! voice {
        ($f:expr, $g:expr) => {
            match wave {
                "sine"             => render_fundsp!(sine_hz::<f32>($f), dur_samples, attack, release, $g),
                "square" | "sq"    => render_fundsp!(square_hz($f),     dur_samples, attack, release, $g),
                "sawtooth" | "saw" => render_fundsp!(saw_hz($f),        dur_samples, attack, release, $g),
                "triangle" | "tri" => render_fundsp!(triangle_hz($f),   dur_samples, attack, release, $g),
                _                  => render_fundsp!(sine_hz::<f32>($f), dur_samples, attack, release, $g),
            }
        };
    }

    // Voice 1: original pitch
    let v1 = voice!(freq, gain);
    // Voice 2: +6 cents detuned, 11 ms delayed — classic 2-osc chorus effect
    let f2    = freq * 2.0f32.powf(6.0 / 1200.0);
    let v2    = voice!(f2, gain * 0.55);
    let delay = (0.011 * SAMPLE_RATE as f32) as usize;

    let mut out = vec![0.0f32; dur_samples];
    for i in 0..dur_samples {
        out[i] = v1[i];
        if i >= delay {
            out[i] += v2[i - delay];
        }
    }

    if matches!(wave, "square" | "sq" | "sawtooth" | "saw") {
        lowpass(&mut out, (freq * 4.0).min(7000.0));
    }
    out
}

// ── Drums (improved) ──────────────────────────────────────────────────────────
//
// Kick:  exponential pitch sweep (sine) + click transient (noise)
// Snare: tone (200 Hz sine) + noise, separate envelopes
// Hat:   high-frequency noise with tight exponential decay
// Clap:  bandpass-style noise burst (multiple short attacks)

fn synth_kick(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = std::cmp::min((0.28 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut phase = 0.0f32;
    let mut lcg = 0xDEAD_BEEFu32;

    (0..dur_samples)
        .map(|i| {
            if i >= cap { return 0.0; }
            let t = i as f32 / SAMPLE_RATE as f32;
            // Pitch sweep 150 → 45 Hz
            let freq = 150.0 * (-t * 28.0).exp() + 45.0;
            let amp_env = (-t * 8.5).exp();
            let tone = (phase * 2.0 * std::f32::consts::PI).sin() * amp_env;
            phase = (phase + freq / SAMPLE_RATE as f32) % 1.0;
            // Click transient: short noise burst in first 5ms
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;
            let click_env = (-t * 200.0).exp();
            let click = noise * click_env * 0.35;
            (tone + click) * gain
        })
        .collect()
}

fn synth_snare(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = std::cmp::min((0.18 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut phase = 0.0f32;
    let mut lcg = 0xDEAD_BEEFu32;

    (0..dur_samples)
        .map(|i| {
            if i >= cap { return 0.0; }
            let t = i as f32 / SAMPLE_RATE as f32;
            // Tonal body: 200 Hz sine, fast decay
            let tone_env = (-t * 35.0).exp();
            let tone = (phase * 2.0 * std::f32::consts::PI).sin() * tone_env * 0.4;
            phase = (phase + 200.0 / SAMPLE_RATE as f32) % 1.0;
            // Noise: slightly longer decay
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise_sample = (lcg >> 16) as f32 / 32_768.0 - 1.0;
            let noise_env = (-t * 18.0).exp();
            let noise = noise_sample * noise_env * 0.7;
            (tone + noise) * gain
        })
        .collect()
}

fn synth_hat(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = std::cmp::min((0.06 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut lcg = 0xFACE_B00Cu32;
    let mut prev = 0.0f32;

    (0..dur_samples)
        .map(|i| {
            if i >= cap { return 0.0; }
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = (-t * 60.0).exp();
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let raw = (lcg >> 16) as f32 / 32_768.0 - 1.0;
            // Simple high-pass: subtract one-pole lowpass from signal
            prev += 0.05 * (raw - prev);
            let hpf = raw - prev;
            hpf * env * gain * 0.6
        })
        .collect()
}

fn synth_clap(dur_samples: usize, gain: f32) -> Vec<f32> {
    let mut lcg = 0xCAFE_BABEu32;
    // Clap = 3 short noise bursts slightly offset, mimics hand-slap body
    let burst_len = (0.008 * SAMPLE_RATE as f32) as usize;
    let gap       = (0.006 * SAMPLE_RATE as f32) as usize;

    (0..dur_samples)
        .map(|i| {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;

            let burst = |start: usize| -> f32 {
                if i >= start && i < start + burst_len {
                    let t = (i - start) as f32 / SAMPLE_RATE as f32;
                    (-t * 80.0).exp()
                } else {
                    0.0
                }
            };

            let env = burst(0) + burst(gap) * 0.7 + burst(gap * 2) * 0.5;
            noise * env * gain
        })
        .collect()
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
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    buf.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes());
    buf.extend_from_slice(&4u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}
