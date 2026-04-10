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
    #[serde(default)]
    pub wave: Option<String>,
    #[serde(default = "default_gain")]
    pub gain: f32,
    #[serde(default)]
    pub pan: Option<f32>,
    #[serde(default)]
    pub attack: Option<f32>,
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

pub fn render_section(pattern: &Pattern) -> Vec<f32> {
    generate_stereo(pattern)
}

// ── Play ──────────────────────────────────────────────────────────────────────

pub fn play_pattern(pattern: &Pattern) -> Result<()> {
    let mut stereo = generate_stereo(pattern);
    trim_trailing_silence(&mut stereo);
    let duration_secs = (stereo.len() / 2) as f32 / SAMPLE_RATE as f32;
    println!(
        "  {:<12}  {} bars · {} BPM · {:.1}s · {} events",
        "synthesizing".truecolor(100, 100, 140),
        pattern.bars, pattern.bpm as u32, duration_secs, pattern.events.len(),
    );
    println!();
    let i16_samples: Vec<i16> = stereo
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();
    let wav = encode_wav(&i16_samples);
    let player = detect_player()?;
    play_wav_bytes(&wav, &player)
}

pub fn play_song(sections: &[(String, Vec<f32>)], _target_secs: u32) -> Result<()> {
    let xfade = (0.040 * SAMPLE_RATE as f32) as usize * 2;
    let mut timeline: Vec<f32> = Vec::new();

    for (_, buf) in sections {
        if timeline.is_empty() {
            timeline.extend_from_slice(buf);
        } else {
            let overlap = (std::cmp::min(xfade, std::cmp::min(buf.len(), timeline.len()))) & !1;
            let start = timeline.len() - overlap;
            for i in 0..overlap {
                let t = i as f32 / overlap as f32;
                // Equal-power crossfade
                let fade_out = (std::f32::consts::FRAC_PI_2 * (1.0 - t)).sin();
                let fade_in = (std::f32::consts::FRAC_PI_2 * t).sin();
                timeline[start + i] = timeline[start + i] * fade_out + buf[i] * fade_in;
            }
            timeline.extend_from_slice(&buf[overlap..]);
        }
    }

    master_bus(&mut timeline);
    trim_trailing_silence(&mut timeline);

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
    let mut stereo = generate_stereo(pattern);
    trim_trailing_silence(&mut stereo);
    let i16_samples: Vec<i16> = stereo
        .iter()
        .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
        .collect();
    encode_wav(&i16_samples)
}

fn generate_stereo(pattern: &Pattern) -> Vec<f32> {
    let beat_secs    = 60.0 / pattern.bpm;
    let total_beats  = pattern.bars as f32 * 4.0;
    let total_samples = ((total_beats * beat_secs + 1.2) * SAMPLE_RATE as f32) as usize;

    let mut mel_l = vec![0.0f32; total_samples];
    let mut mel_r = vec![0.0f32; total_samples];
    let mut drm_l = vec![0.0f32; total_samples];
    let mut drm_r = vec![0.0f32; total_samples];
    let mut bass_l = vec![0.0f32; total_samples];
    let mut bass_r = vec![0.0f32; total_samples];

    // Track kick positions for sidechain ducking on bass
    let mut kick_times: Vec<usize> = Vec::new();

    for event in &pattern.events {
        let start = (event.t * beat_secs * SAMPLE_RATE as f32) as usize;
        let dur_samples = std::cmp::max(
            (event.dur * beat_secs * SAMPLE_RATE as f32) as usize, 1,
        );
        let drum = is_drum_note(&event.note);
        let note_lower = event.note.to_lowercase();

        if note_lower == "kick" || note_lower == "bd" {
            kick_times.push(start);
        }

        let samples = synth_event(event, dur_samples);

        let pan   = event.pan.unwrap_or(0.0).clamp(-1.0, 1.0);
        let angle = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        let l_gain = angle.cos();
        let r_gain = angle.sin();

        // Route bass notes to separate bus for sidechain processing
        let is_bass = !drum && note_to_freq(&event.note)
            .map(|f| f < 200.0)
            .unwrap_or(false);

        let (tl, tr) = if drum {
            (&mut drm_l, &mut drm_r)
        } else if is_bass {
            (&mut bass_l, &mut bass_r)
        } else {
            (&mut mel_l, &mut mel_r)
        };

        for (i, &s) in samples.iter().enumerate() {
            let idx = start + i;
            if idx < total_samples {
                tl[idx] += s * l_gain;
                tr[idx] += s * r_gain;
            }
        }
    }

    // Sidechain duck: bass dips when kick hits
    apply_sidechain(&mut bass_l, &kick_times);
    apply_sidechain(&mut bass_r, &kick_times);

    // Reverb on melodic bus only; wetness LLM-controlled
    let wet = pattern.reverb.unwrap_or(0.22).clamp(0.0, 1.0);
    let mel_l = apply_reverb(&mel_l, wet, 0);
    let mel_r = apply_reverb(&mel_r, wet, 1);

    // Subtle room verb on drums for cohesion
    let drm_verb = (wet * 0.15).clamp(0.0, 0.12);
    let drm_l = apply_reverb(&drm_l, drm_verb, 0);
    let drm_r = apply_reverb(&drm_r, drm_verb, 1);

    // Mix all buses
    let mut left:  Vec<f32> = Vec::with_capacity(total_samples);
    let mut right: Vec<f32> = Vec::with_capacity(total_samples);
    for i in 0..total_samples {
        left.push(mel_l[i] * 0.80 + bass_l[i] * 0.90 + drm_l[i] * 0.95);
        right.push(mel_r[i] * 0.80 + bass_r[i] * 0.90 + drm_r[i] * 0.95);
    }

    // DC offset removal
    dc_block(&mut left);
    dc_block(&mut right);

    // Soft-clip limiter instead of hard normalization
    let peak = left.iter().chain(right.iter())
        .fold(0.0f32, |a, s| a.max(s.abs()));
    if peak > 0.0 {
        let target = 0.88;
        let scale = if peak > target { target / peak } else { 1.0 };
        for s in left.iter_mut().chain(right.iter_mut()) {
            *s = soft_clip(*s * scale);
        }
    }

    // Fade in/out to prevent clicks at section boundaries
    let fade = std::cmp::min((0.015 * SAMPLE_RATE as f32) as usize, left.len() / 4);
    let left_len = left.len();
    let right_len = right.len();
    for i in 0..fade {
        let t = (i as f32 / fade as f32).powi(2);
        left[i]  *= t;  right[i]  *= t;
        left[left_len  - 1 - i] *= t;
        right[right_len - 1 - i] *= t;
    }

    left.iter().zip(right.iter()).flat_map(|(&l, &r)| [l, r]).collect()
}

/// Trim trailing silence from stereo-interleaved samples.
/// Keeps a 50ms fade-out tail after the last audible sample.
fn trim_trailing_silence(stereo: &mut Vec<f32>) {
    let threshold = 0.0015; // ~-56dB — below audible
    let tail = (0.05 * SAMPLE_RATE as f32) as usize * 2; // 50ms stereo tail

    // Scan backwards for last audible stereo pair
    let mut last_audible = 0;
    let len = stereo.len();
    let mut i = len;
    while i >= 2 {
        i -= 2;
        if stereo[i].abs() > threshold || stereo[i + 1].abs() > threshold {
            last_audible = i + 2;
            break;
        }
    }

    if last_audible == 0 {
        return;
    }

    let end = std::cmp::min(last_audible + tail, len) & !1; // keep stereo-aligned
    stereo.truncate(end);

    // Gentle fade-out on the tail portion
    let fade_len = std::cmp::min(tail, end - last_audible) / 2;
    if fade_len > 0 {
        for j in 0..fade_len {
            let t = 1.0 - (j as f32 / fade_len as f32);
            let idx = last_audible + j * 2;
            if idx + 1 < stereo.len() {
                stereo[idx] *= t;
                stereo[idx + 1] *= t;
            }
        }
    }
}

/// Master bus processing for full songs (applied to interleaved stereo)
fn master_bus(stereo: &mut [f32]) {
    let len = stereo.len() / 2;
    let peak = stereo.iter().fold(0.0f32, |a, s| a.max(s.abs()));
    if peak > 0.0 {
        let target = 0.88;
        let scale = if peak > target { target / peak } else { 1.0 };
        for s in stereo.iter_mut() {
            *s = soft_clip(*s * scale);
        }
    }

    // DC block on each channel
    let mut dc_l = 0.0f32;
    let mut dc_r = 0.0f32;
    for i in 0..len {
        let l = stereo[i * 2];
        let r = stereo[i * 2 + 1];
        dc_l = 0.999 * dc_l + l - dc_l;
        dc_r = 0.999 * dc_r + r - dc_r;
        stereo[i * 2] = l - dc_l * 0.001;
        stereo[i * 2 + 1] = r - dc_r * 0.001;
    }
}

fn is_drum_note(note: &str) -> bool {
    matches!(
        note.to_lowercase().as_str(),
        "kick" | "bd" | "snare" | "sd" | "hat" | "hh" | "hihat"
        | "openhat" | "oh" | "clap" | "cp"
        | "rim" | "rimshot" | "rs"
        | "tom" | "tomhi" | "tomlo" | "tommid"
        | "ride" | "rd" | "crash" | "cr"
    )
}

// ── Sidechain ducking ─────────────────────────────────────────────────────────

fn apply_sidechain(bus: &mut [f32], kick_times: &[usize]) {
    let duck_samples = (0.08 * SAMPLE_RATE as f32) as usize; // 80ms duck
    let release_samples = (0.12 * SAMPLE_RATE as f32) as usize; // 120ms release

    for &kick_start in kick_times {
        let total = duck_samples + release_samples;
        for i in 0..total {
            let idx = kick_start + i;
            if idx >= bus.len() { break; }

            let duck = if i < duck_samples {
                let t = i as f32 / duck_samples as f32;
                // Quick exponential duck to -6dB
                1.0 - 0.5 * (1.0 - (t * 3.0).min(1.0))
            } else {
                let t = (i - duck_samples) as f32 / release_samples as f32;
                0.5 + 0.5 * t.powi(2)
            };
            bus[idx] *= duck;
        }
    }
}

// ── DC offset removal ─────────────────────────────────────────────────────────

fn dc_block(samples: &mut [f32]) {
    let mut xm1 = 0.0f32;
    let mut ym1 = 0.0f32;
    let r = 0.9975; // ~5Hz high-pass at 44.1kHz
    for s in samples.iter_mut() {
        let x = *s;
        let y = x - xm1 + r * ym1;
        xm1 = x;
        ym1 = y;
        *s = y;
    }
}

// ── Freeverb (8 comb + 4 allpass) ─────────────────────────────────────────────

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

/// Freeverb-quality reverb. `channel` offsets delay times for stereo spread.
fn apply_reverb(input: &[f32], wet: f32, channel: usize) -> Vec<f32> {
    let sr = SAMPLE_RATE as f32;

    // Freeverb delay times (in seconds), with stereo spread offset
    let spread = if channel == 0 { 0 } else { 23 };
    let comb_delays = [
        (sr * 0.0297) as usize + spread,
        (sr * 0.0371) as usize + spread,
        (sr * 0.0411) as usize + spread,
        (sr * 0.0437) as usize + spread,
        (sr * 0.0482) as usize + spread,
        (sr * 0.0513) as usize + spread,
        (sr * 0.0557) as usize + spread,
        (sr * 0.0617) as usize + spread,
    ];

    // Feedback and damping scale with wetness for natural decay
    let room = 0.70 + wet * 0.18;
    let damp = 0.35 + (1.0 - wet) * 0.20;

    let mut combs: Vec<CombFilter> = comb_delays
        .iter()
        .map(|&d| CombFilter::new(d, room, damp))
        .collect();

    let ap_spread = if channel == 0 { 0 } else { 12 };
    let mut allpasses = [
        AllPassFilter::new((sr * 0.0050) as usize + ap_spread),
        AllPassFilter::new((sr * 0.0126) as usize + ap_spread),
        AllPassFilter::new((sr * 0.0100) as usize + ap_spread),
        AllPassFilter::new((sr * 0.0077) as usize + ap_spread),
    ];

    // Pre-delay: ~15ms for sense of space
    let predelay = (0.015 * sr) as usize;

    let dry = 1.0 - wet;
    let gain = 0.015; // input attenuation to combs

    input
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let delayed = if i >= predelay { input[i - predelay] } else { 0.0 };

            let verb: f32 = combs.iter_mut().map(|c| c.process(delayed * gain)).sum();

            let mut out = verb;
            for ap in allpasses.iter_mut() {
                out = ap.process(out);
            }

            s * dry + out * wet * 3.0
        })
        .collect()
}

// ── Biquad low-pass filter (12dB/oct) ─────────────────────────────────────────

fn biquad_lowpass(samples: &mut [f32], cutoff_hz: f32) {
    let sr = SAMPLE_RATE as f32;
    let w0 = std::f32::consts::TAU * cutoff_hz / sr;
    let q = 0.707f32; // Butterworth
    let alpha = w0.sin() / (2.0 * q);
    let cos_w0 = w0.cos();

    let b0 = (1.0 - cos_w0) / 2.0;
    let b1 = 1.0 - cos_w0;
    let b2 = b0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha;

    let b0 = b0 / a0;
    let b1 = b1 / a0;
    let b2 = b2 / a0;
    let a1 = a1 / a0;
    let a2 = a2 / a0;

    let mut x1 = 0.0f32;
    let mut x2 = 0.0f32;
    let mut y1 = 0.0f32;
    let mut y2 = 0.0f32;

    for s in samples.iter_mut() {
        let x0 = *s;
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        x2 = x1;
        x1 = x0;
        y2 = y1;
        y1 = y0;
        *s = y0;
    }
}

#[allow(dead_code)]
fn biquad_highpass(samples: &mut [f32], cutoff_hz: f32) {
    let sr = SAMPLE_RATE as f32;
    let w0 = std::f32::consts::TAU * cutoff_hz / sr;
    let q = 0.707f32;
    let alpha = w0.sin() / (2.0 * q);
    let cos_w0 = w0.cos();

    let b0 = (1.0 + cos_w0) / 2.0;
    let b1 = -(1.0 + cos_w0);
    let b2 = b0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha;

    let b0 = b0 / a0;
    let b1 = b1 / a0;
    let b2 = b2 / a0;
    let a1 = a1 / a0;
    let a2 = a2 / a0;

    let mut x1 = 0.0f32;
    let mut x2 = 0.0f32;
    let mut y1 = 0.0f32;
    let mut y2 = 0.0f32;

    for s in samples.iter_mut() {
        let x0 = *s;
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        x2 = x1;
        x1 = x0;
        y2 = y1;
        y1 = y0;
        *s = y0;
    }
}

// ── Event dispatch ────────────────────────────────────────────────────────────

fn synth_event(event: &MusicEvent, dur_samples: usize) -> Vec<f32> {
    match event.note.to_lowercase().as_str() {
        "kick" | "bd"       => synth_kick(dur_samples, event.gain),
        "snare" | "sd"      => synth_snare(dur_samples, event.gain),
        "hat" | "hh" | "hihat" => synth_hat(dur_samples, event.gain),
        "openhat" | "oh"    => synth_open_hat(dur_samples, event.gain),
        "clap" | "cp"       => synth_clap(dur_samples, event.gain),
        "rim" | "rimshot" | "rs" => synth_rim(dur_samples, event.gain),
        "tom"               => synth_tom(dur_samples, event.gain, 110.0),
        "tomhi"             => synth_tom(dur_samples, event.gain, 165.0),
        "tomlo"             => synth_tom(dur_samples, event.gain, 80.0),
        "tommid"            => synth_tom(dur_samples, event.gain, 110.0),
        "ride" | "rd"       => synth_ride(dur_samples, event.gain),
        "crash" | "cr"      => synth_crash(dur_samples, event.gain),
        _ => match note_to_freq(&event.note) {
            Some(freq) => {
                let wave = event.wave.as_deref().unwrap_or("sine");
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

// ── Melodic oscillator (3-voice chorus + vibrato + sub-osc) ───────────────────

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

    // Voice 1: original pitch
    let v1 = voice!(freq, gain);

    // Voice 2: +7 cents, 11ms delayed — classic stereo chorus
    let f2 = freq * 2.0f32.powf(7.0 / 1200.0);
    let v2 = voice!(f2, gain * 0.45);
    let delay2 = (0.011 * SAMPLE_RATE as f32) as usize;

    // Voice 3: -5 cents, 7ms delayed — wider stereo image
    let f3 = freq * 2.0f32.powf(-5.0 / 1200.0);
    let v3 = voice!(f3, gain * 0.30);
    let delay3 = (0.007 * SAMPLE_RATE as f32) as usize;

    let mut out = vec![0.0f32; dur_samples];

    // Subtle vibrato: sinusoidal pitch modulation via amplitude modulation of voices
    let vibrato_rate = 5.0; // Hz
    let vibrato_depth = 0.008; // subtle

    for i in 0..dur_samples {
        let vib = 1.0 + (i as f32 / SAMPLE_RATE as f32 * vibrato_rate * std::f32::consts::TAU).sin() * vibrato_depth;
        out[i] = v1[i] * vib;
        if i >= delay2 { out[i] += v2[i - delay2]; }
        if i >= delay3 { out[i] += v3[i - delay3]; }
    }

    // Sub-oscillator for bass frequencies (one octave down, sine only)
    if freq < 200.0 {
        let sub = voice!(freq * 0.5, gain * 0.30);
        for i in 0..dur_samples {
            out[i] += sub[i];
        }
    }

    if matches!(wave, "square" | "sq" | "sawtooth" | "saw") {
        biquad_lowpass(&mut out, (freq * 5.0).min(8000.0));
    }

    out
}

// ── Drums ─────────────────────────────────────────────────────────────────────

fn synth_kick(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = std::cmp::min((0.45 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut phase_body = 0.0f32;
    let mut phase_sub = 0.0f32;
    let mut lcg = 0xDEAD_BEEFu32;

    let mut out: Vec<f32> = (0..dur_samples)
        .map(|i| {
            if i >= cap { return 0.0; }
            let t = i as f32 / SAMPLE_RATE as f32;

            // Body: pitch-swept sine 160→50 Hz with punch
            let body_freq = 120.0 * (-t * 35.0).exp() + 50.0;
            let body = (phase_body * std::f32::consts::TAU).sin();
            let body_env = (-t * 7.0).exp();
            phase_body = (phase_body + body_freq / SAMPLE_RATE as f32) % 1.0;

            // Sub: sustained low sine at 48 Hz for weight
            let sub = (phase_sub * std::f32::consts::TAU).sin();
            let sub_env = (-t * 4.5).exp();
            phase_sub = (phase_sub + 48.0 / SAMPLE_RATE as f32) % 1.0;

            // Transient click
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;
            let click = noise * (-t * 300.0).exp() * 0.30;

            let raw = body * body_env * 0.70 + sub * sub_env * 0.40 + click;
            soft_clip(raw) * gain
        })
        .collect();

    biquad_lowpass(&mut out, 120.0);
    out
}

fn synth_snare(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = std::cmp::min((0.30 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut phase1 = 0.0f32;
    let mut phase2 = 0.0f32;
    let mut lcg = 0xDEAD_BEEFu32;
    let mut bp_state = [0.0f32; 2];

    (0..dur_samples)
        .map(|i| {
            if i >= cap { return 0.0; }
            let t = i as f32 / SAMPLE_RATE as f32;

            // Tonal body: two sine partials at ~185 Hz and ~330 Hz
            let tone1 = (phase1 * std::f32::consts::TAU).sin() * (-t * 22.0).exp();
            phase1 = (phase1 + 185.0 / SAMPLE_RATE as f32) % 1.0;
            let tone2 = (phase2 * std::f32::consts::TAU).sin() * (-t * 28.0).exp();
            phase2 = (phase2 + 330.0 / SAMPLE_RATE as f32) % 1.0;

            // Noise layer: bandpass-filtered for "snare wire" character
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let raw_noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;
            let noise_env = (-t * 14.0).exp();

            let fc = 3500.0 / SAMPLE_RATE as f32;
            let w0 = std::f32::consts::TAU * fc;
            let alpha = w0.sin() / 2.4;
            let filtered = (raw_noise - bp_state[1]) * alpha + bp_state[0];
            bp_state[0] = filtered;
            bp_state[1] = bp_state[1] + alpha * filtered;
            let noise = filtered * noise_env * 0.85;

            // Sharp transient hit
            let transient = raw_noise * (-t * 180.0).exp() * 0.25;

            (tone1 * 0.35 + tone2 * 0.20 + noise + transient) * gain
        })
        .collect()
}

fn synth_hat(dur_samples: usize, gain: f32) -> Vec<f32> {
    synth_hat_inner(dur_samples, gain, 55.0)
}

fn synth_open_hat(dur_samples: usize, gain: f32) -> Vec<f32> {
    synth_hat_inner(dur_samples, gain, 8.0)
}

fn synth_hat_inner(dur_samples: usize, gain: f32, decay_rate: f32) -> Vec<f32> {
    let cap = if decay_rate < 20.0 {
        std::cmp::min((0.35 * SAMPLE_RATE as f32) as usize, dur_samples)
    } else {
        std::cmp::min((0.08 * SAMPLE_RATE as f32) as usize, dur_samples)
    };

    // Metallic inharmonic partials (like real cymbal modes)
    let freqs = [3578.0f32, 4721.0, 5765.0, 6812.0, 8856.0, 10245.0];
    let mut phases = [0.0f32; 6];
    let mut lcg = 0xFACE_B00Cu32;
    let mut hp_state = 0.0f32;

    (0..dur_samples)
        .map(|i| {
            if i >= cap { return 0.0; }
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = (-t * decay_rate).exp();

            let mut ring = 0.0f32;
            for (j, &freq) in freqs.iter().enumerate() {
                let amp = 1.0 / (j as f32 + 1.0);
                ring += (phases[j] * std::f32::consts::TAU).sin() * amp;
                phases[j] = (phases[j] + freq / SAMPLE_RATE as f32) % 1.0;
            }
            ring *= 0.15;

            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;

            let mixed = (ring + noise * 0.55) * env;

            // High-pass to remove low-end rumble
            let hp = mixed - hp_state;
            hp_state += 0.015 * hp;

            hp * gain * 0.55
        })
        .collect()
}

fn synth_clap(dur_samples: usize, gain: f32) -> Vec<f32> {
    let mut lcg = 0xCAFE_BABEu32;
    let burst = (0.006 * SAMPLE_RATE as f32) as usize;
    let gap = (0.008 * SAMPLE_RATE as f32) as usize;
    let cap = std::cmp::min((0.25 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut bp_state = [0.0f32; 2];

    (0..dur_samples)
        .map(|i| {
            if i >= cap { return 0.0; }
            let t = i as f32 / SAMPLE_RATE as f32;
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let raw_noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;

            // Bandpass around 1.2 kHz for body
            let fc = 1200.0 / SAMPLE_RATE as f32;
            let w0 = std::f32::consts::TAU * fc;
            let alpha = w0.sin() / 3.0;
            let filtered = (raw_noise - bp_state[1]) * alpha + bp_state[0];
            bp_state[0] = filtered;
            bp_state[1] = bp_state[1] + alpha * filtered;

            // 4 micro-bursts with decaying amplitude
            let b = |start: usize, amp: f32| -> f32 {
                if i >= start && i < start + burst {
                    let bt = (i - start) as f32 / SAMPLE_RATE as f32;
                    (-bt * 120.0).exp() * amp
                } else {
                    0.0
                }
            };
            let bursts = b(0, 1.0) + b(gap, 0.75) + b(gap * 2, 0.55) + b(gap * 3, 0.40);

            // Filtered tail
            let tail = filtered * (-t * 20.0).exp() * 0.4;

            (raw_noise * bursts + tail) * gain
        })
        .collect()
}

fn synth_rim(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = std::cmp::min((0.05 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut phase = 0.0f32;

    (0..dur_samples)
        .map(|i| {
            if i >= cap { return 0.0; }
            let t = i as f32 / SAMPLE_RATE as f32;
            let tone = (phase * std::f32::consts::TAU).sin();
            phase = (phase + 820.0 / SAMPLE_RATE as f32) % 1.0;
            tone * (-t * 90.0).exp() * gain * 0.7
        })
        .collect()
}

fn synth_tom(dur_samples: usize, gain: f32, base_freq: f32) -> Vec<f32> {
    let cap = std::cmp::min((0.35 * SAMPLE_RATE as f32) as usize, dur_samples);
    let mut phase = 0.0f32;

    (0..dur_samples)
        .map(|i| {
            if i >= cap { return 0.0; }
            let t = i as f32 / SAMPLE_RATE as f32;
            let freq = base_freq * 1.5 * (-t * 15.0).exp() + base_freq;
            let tone = (phase * std::f32::consts::TAU).sin();
            phase = (phase + freq / SAMPLE_RATE as f32) % 1.0;
            soft_clip(tone * (-t * 6.0).exp() * 1.2) * gain
        })
        .collect()
}

fn synth_ride(dur_samples: usize, gain: f32) -> Vec<f32> {
    synth_hat_inner(dur_samples, gain * 0.65, 4.0)
}

fn synth_crash(dur_samples: usize, gain: f32) -> Vec<f32> {
    let cap = std::cmp::min((1.2 * SAMPLE_RATE as f32) as usize, dur_samples);
    let freqs = [2043.0f32, 3521.0, 4895.0, 6231.0, 7654.0, 9120.0, 11200.0];
    let mut phases = [0.0f32; 7];
    let mut lcg = 0xDEAD_CAFEu32;
    let mut hp_state = 0.0f32;

    (0..dur_samples)
        .map(|i| {
            if i >= cap { return 0.0; }
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = (-t * 2.5).exp();

            let mut ring = 0.0f32;
            for (j, &freq) in freqs.iter().enumerate() {
                let amp = 1.0 / (j as f32 * 0.7 + 1.0);
                ring += (phases[j] * std::f32::consts::TAU).sin() * amp;
                phases[j] = (phases[j] + freq / SAMPLE_RATE as f32) % 1.0;
            }
            ring *= 0.10;

            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (lcg >> 16) as f32 / 32_768.0 - 1.0;

            let mixed = (ring + noise * 0.45) * env;
            let hp = mixed - hp_state;
            hp_state += 0.01 * hp;

            hp * gain * 0.50
        })
        .collect()
}

fn soft_clip(x: f32) -> f32 {
    if x.abs() < 0.5 {
        x
    } else {
        x.signum() * (1.0 - (-x.abs() * 2.0).exp()) * 0.75 + x * 0.25
    }
}

// ── ADSR envelope (exponential curves) ────────────────────────────────────────

fn adsr_env(i: usize, total: usize, attack: usize, release: usize) -> f32 {
    if attack == 0 && release == 0 {
        return 1.0;
    }

    if i < attack {
        // Exponential attack: fast initial rise, smooth arrival at 1.0
        let t = i as f32 / attack as f32;
        1.0 - (-t * 4.0).exp()
    } else if total > release && i >= total - release {
        // Exponential release: natural decay
        let t = (i - (total - release)) as f32 / release as f32;
        (-t * 5.0).exp()
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
