pub mod cerebras;
pub mod cohere;

use anyhow::Result;

pub const STRUDEL_SYSTEM_PROMPT: &str = r#"You are a music composer for a CLI audio synthesizer. Generate a musical pattern in this EXACT JSON format — no markdown, no explanation, just the JSON object:

{
  "bpm": <integer 70-160>,
  "bars": <integer 2-8, how many bars to generate>,
  "events": [
    {"t": <float beats>, "dur": <float beats>, "note": "<note or drum>", "wave": "<wave>", "gain": <float>}
  ]
}

TIMING:
- 4/4 time. t=0 is bar 1 beat 1. Beat 2 = t:1, beat 3 = t:2, beat 4 = t:3. Bar 2 starts at t:4.
- dur: 0.25 = sixteenth, 0.5 = eighth, 1.0 = quarter, 2.0 = half, 4.0 = whole

NOTES (melodic):
- Format: "C4", "Eb3", "F#5", "Bb4" (sharps=#, flats=b, octave 2–5)
- Bass lines: octave 2–3 | Melody: octave 4–5 | Pads/chords: octave 3–4
- wave: "sine" (default, warm), "triangle" (hollow), "square" (harsh), "sawtooth" (bright)
- gain: 0.3–0.7 for melodic

DRUMS:
- "kick"  → bass drum — place on beats 1 and 3 (t: 0, 2, 4, 6 ...)
- "snare" → snare drum — place on beats 2 and 4 (t: 1, 3, 5, 7 ...)
- "hat"   → hi-hat — subdivisions (t: 0, 0.5, 1, 1.5 ... for eighth notes)
- "clap"  → clap — add for texture
- gain: kick=0.85, snare=0.7, hat=0.45, clap=0.55

ARRANGEMENT: Create a rich, layered pattern with bass + melody + drums (at minimum). Add pads/chords for depth. 4 bars is a good default length.

Return ONLY the JSON object. No extra text."#;

pub struct LlmRequest {
    pub prompt: String,
    pub model: String,
    pub api_key: String,
}

pub async fn generate_strudel(request: LlmRequest, provider: &crate::cli::Provider) -> Result<String> {
    let raw = match provider {
        crate::cli::Provider::Cerebras => cerebras::complete(&request).await?,
        crate::cli::Provider::Cohere => cohere::complete(&request).await?,
    };
    Ok(extract_json(raw))
}

/// Strip markdown fences and find the JSON object even if the LLM added extra text.
fn extract_json(raw: String) -> String {
    let s = raw.trim();
    // Strip ```json ... ``` or ``` ... ```
    let s = s
        .strip_prefix("```json")
        .or_else(|| s.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .map(|s| s.trim())
        .unwrap_or(s);

    // Find the outermost JSON object
    if let Some(start) = s.find('{') {
        if let Some(end) = s.rfind('}') {
            return s[start..=end].to_string();
        }
    }
    s.to_string()
}
