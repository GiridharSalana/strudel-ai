pub mod cerebras;
pub mod cohere;

use anyhow::{Context, Result};

// ── Base JSON format used by every section call ───────────────────────────────

pub const FORMAT_RULES: &str = r#"Output a JSON object with this exact schema — no markdown fences, no extra text:
{
  "bpm": <integer 60-160>,
  "bars": <integer>,
  "events": [
    {"t": <beat float>, "dur": <beat float>, "note": "<pitch or drum>", "wave": "<waveform>", "gain": <0.1-0.9>}
  ]
}

TIMING: 4/4 time. t=0 is bar 1 beat 1; t=4 is bar 2 beat 1. Beat = 1 unit.
NOTES: "C2"–"B5" (e.g. "C4", "Eb3", "F#5"). Bass in octaves 2–3, melody in 4–5.
DRUMS: "kick" (beats 1,3), "snare" (beats 2,4), "hat" (subdivisions), "clap"
WAVE (melodic only): "sine", "triangle", "square", "sawtooth"
GAIN: kick=0.85, snare=0.70, hat=0.45, melody=0.50, bass=0.60, pad=0.35
Return ONLY the JSON object."#;

// ── Single-pattern prompt (short loop mode) ───────────────────────────────────

pub const STRUDEL_SYSTEM_PROMPT: &str = r#"You are a music composer for a CLI audio synthesizer. Generate a musical pattern.
Create a rich, layered arrangement with bass + melody + drums. 4 bars is a good default.

ARRANGEMENT: kick on beats 1,3 · snare on beats 2,4 · hat subdivisions · bass line · melodic phrases · optional pads

Output a JSON object with this exact schema — no markdown fences, no extra text:
{
  "bpm": <integer 60-160>,
  "bars": <integer 2-8>,
  "events": [
    {"t": <beat float>, "dur": <beat float>, "note": "<pitch or drum>", "wave": "<waveform>", "gain": <0.1-0.9>}
  ]
}

TIMING: 4/4 time. t=0 = bar 1 beat 1; t=4 = bar 2 beat 1.
NOTES: "C2"–"B5". Bass: octaves 2–3. Melody: 4–5.
DRUMS: "kick", "snare", "hat", "clap"
WAVE: "sine", "triangle", "square", "sawtooth"
Return ONLY the JSON object."#;

// ── Section definitions for full-song mode ────────────────────────────────────

pub struct SectionSpec {
    pub name: &'static str,
    pub bars: u32,
    pub role: &'static str,
}

pub const SONG_SECTIONS: &[SectionSpec] = &[
    SectionSpec {
        name: "intro",
        bars: 8,
        role: "INTRO — Sparse and atmospheric. Start with just bass and light percussion. \
               No melody yet. Build anticipation. Low energy.",
    },
    SectionSpec {
        name: "verse_a",
        bars: 16,
        role: "VERSE A — Main melodic theme. Medium energy. Bass + melody + drums. \
               The core groove of the track.",
    },
    SectionSpec {
        name: "verse_b",
        bars: 16,
        role: "VERSE B — Variation of verse A. Use the same BPM and key but change \
               the melodic phrase or add a counter-melody. Slightly higher energy.",
    },
    SectionSpec {
        name: "chorus",
        bars: 8,
        role: "CHORUS — The hook. Full energy. All elements playing together — bass, \
               melody, drums, pads. This is the emotional peak.",
    },
    SectionSpec {
        name: "bridge",
        bars: 8,
        role: "BRIDGE — Contrast. Break from the main progression. Sparse or rhythmically \
               different. Creates tension before the final chorus.",
    },
    SectionSpec {
        name: "outro",
        bars: 8,
        role: "OUTRO — Wind down. Mirror the intro. Remove elements gradually. \
               End quietly. Same BPM, fading energy.",
    },
];

// ── Request / response types ──────────────────────────────────────────────────

pub struct LlmRequest {
    pub prompt: String,
    pub model: String,
    pub api_key: String,
}

// ── Single pattern generation ─────────────────────────────────────────────────

pub async fn generate_strudel(
    request: LlmRequest,
    provider: &crate::cli::Provider,
) -> Result<String> {
    let raw = match provider {
        crate::cli::Provider::Cerebras => cerebras::complete(&request).await?,
        crate::cli::Provider::Cohere => cohere::complete(&request).await?,
    };
    Ok(extract_json(raw))
}

// ── Multi-section song generation ─────────────────────────────────────────────

/// Generates all song sections by making one LLM call per section.
/// Returns (section_name, pattern_json) pairs in order.
pub async fn generate_sections(
    style_prompt: &str,
    provider: &crate::cli::Provider,
    api_key: &str,
    model: &str,
) -> Result<Vec<(String, String)>> {
    let mut results: Vec<(String, String)> = Vec::new();
    let mut established_bpm: Option<u32> = None;

    for spec in SONG_SECTIONS {
        let bpm_line = match established_bpm {
            Some(b) => format!("Use exactly BPM: {}. Do not change the tempo.", b),
            None    => "Choose an appropriate BPM for the style (60–160).".to_string(),
        };

        let prompt = format!(
            "Style: {style_prompt}\n\
             Section: {} ({} bars)\n\
             Role: {}\n\
             {bpm_line}\n\n\
             {FORMAT_RULES}",
            spec.name, spec.bars, spec.role
        );

        let req = LlmRequest {
            prompt,
            model: model.to_string(),
            api_key: api_key.to_string(),
        };

        let raw = match provider {
            crate::cli::Provider::Cerebras => cerebras::complete(&req).await?,
            crate::cli::Provider::Cohere   => cohere::complete(&req).await?,
        };
        let json = extract_json(raw);

        // Extract BPM from the first section to lock it for the rest
        if established_bpm.is_none() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json) {
                if let Some(b) = val.get("bpm").and_then(|v| v.as_u64()) {
                    established_bpm = Some(b as u32);
                }
            }
        }

        results.push((spec.name.to_string(), json));
    }

    Ok(results)
}

// ── Build an arrangement that fills target_secs ───────────────────────────────

/// Returns a Vec of section names in playback order.
pub fn build_arrangement(bpm: f32, target_secs: u32) -> Vec<String> {
    let beat_secs = 60.0 / bpm;
    let bar_secs  = beat_secs * 4.0;

    // How long each section type is (matching SONG_SECTIONS bars)
    let intro_secs   = 8.0  * bar_secs;
    let verse_secs   = 16.0 * bar_secs;
    let chorus_secs  = 8.0  * bar_secs;
    let bridge_secs  = 8.0  * bar_secs;
    let outro_secs   = 8.0  * bar_secs;

    // Fixed end: bridge + chorus + outro
    let end_secs = bridge_secs + chorus_secs + outro_secs;

    let mut arrangement = vec!["intro".to_string()];
    let mut elapsed = intro_secs;
    let mut verse_flip = true;

    // Fill the middle with alternating verse A/B + chorus
    loop {
        let verse = if verse_flip { "verse_a" } else { "verse_b" };
        let needed = verse_secs + chorus_secs + end_secs;

        if elapsed + needed > target_secs as f32 {
            break;
        }

        arrangement.push(verse.to_string());
        elapsed += verse_secs;
        verse_flip = !verse_flip;

        arrangement.push("chorus".to_string());
        elapsed += chorus_secs;
    }

    // If we never got a verse+chorus in (very short duration), add at least one
    if arrangement.len() == 1 {
        arrangement.push("verse_a".to_string());
        arrangement.push("chorus".to_string());
    }

    arrangement.push("bridge".to_string());
    arrangement.push("chorus".to_string());
    arrangement.push("outro".to_string());

    arrangement
}

// ── JSON extraction ───────────────────────────────────────────────────────────

pub fn extract_json(raw: String) -> String {
    let s = raw.trim();
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .map(|s| s.trim())
        .unwrap_or(s);

    if let Some(start) = s.find('{') {
        if let Some(end) = s.rfind('}') {
            return s[start..=end].to_string();
        }
    }
    s.to_string()
}
