pub mod cerebras;
pub mod cohere;

// ── Base JSON format used by every section call ───────────────────────────────

pub const FORMAT_RULES: &str = r#"You are both composer AND sound engineer. Every musical and sonic parameter is yours to decide.

Output ONLY a JSON object — no markdown fences, no extra text:
{
  "bpm": <integer 60-160>,
  "bars": <integer>,
  "reverb": <0.0-1.0>,
  "events": [
    {
      "t": <beat>,   "dur": <beat>,   "note": "<pitch or drum>",
      "wave": "<waveform>",
      "gain": <0.1-0.9>,
      "pan": <-1.0 to 1.0>,
      "attack": <seconds>,
      "release": <seconds>
    }
  ]
}

━━ TIMING ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
4/4 time. t=0 = bar 1 beat 1.  t=4 = bar 2 beat 1.  Beat = 1 unit.
Fill ALL requested bars with events that span the full duration.

━━ NOTES ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Pitches: "C2"–"B5"  (e.g. "C4", "Eb3", "F#5", "Bb4")
Bass lines: octaves 2–3 · Melody / lead: 4–5 · Pads: 3–4

━━ DRUMS ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
"kick"   — thump on beats 1, 3
"snare"  — crack on beats 2, 4
"hat"    — hi-hat subdivisions (8ths, 16ths)
"clap"   — accent (can double the snare or offbeat)
"openhat"— open hi-hat for longer sustain

━━ WAVE (melodic only) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
"sine"      smooth, pure, warm
"triangle"  softer version of square, gentle
"square"    hollow, woody, lo-fi organ
"sawtooth"  bright, buzzy, classic synth lead/bass

━━ GAIN ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Kick 0.80–0.90 · Snare 0.65–0.75 · Hat 0.35–0.50 · Clap 0.60–0.70
Bass 0.55–0.70 · Lead melody 0.45–0.60 · Pad/chord 0.25–0.40

━━ PAN ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
-1.0 hard-left · 0.0 center · 1.0 hard-right
Kick, bass, snare → center (0.0)
Hi-hat → slight right (0.3–0.5)
Clap → slight left (-0.2)
Lead melody → slight left or right (-0.3 to 0.3)
Pad chord voices → spread wide (e.g. -0.6 and +0.6)

━━ ATTACK & RELEASE (melodic only, in seconds) ━━━━━━━━━━━━
attack:  0.001 pluck · 0.01 normal · 0.05–0.2 pad swell
release: 0.05 staccato · 0.15 normal · 0.4–1.0 sustained/pad

━━ REVERB (whole pattern) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
0.0 completely dry · 0.15 small room · 0.3 medium hall
0.5 large hall · 0.7 cathedral · 0.9 huge ambient wash
(applied to melodic layer only; drums stay dry)"#;

// ── Single-pattern prompt (loop mode) ────────────────────────────────────────

pub const STRUDEL_SYSTEM_PROMPT: &str = r#"You are composer AND sound engineer. Generate a complete, self-contained musical piece for a CLI synthesizer.
Use 8–16 bars. The piece plays exactly once — make it feel complete with a beginning, development, and resolution.
Layer bass + melody + drums, and use panning, reverb, and ADSR to shape a professional mix.

Output ONLY a JSON object — no markdown fences, no extra text. Use the full schema:
{
  "bpm": <integer 60-160>,
  "bars": <integer 8-16>,
  "reverb": <0.0-1.0>,
  "events": [
    {
      "t": <beat>, "dur": <beat>, "note": "<pitch or drum>",
      "wave": "<waveform>", "gain": <0.1-0.9>,
      "pan": <-1.0 to 1.0>, "attack": <seconds>, "release": <seconds>
    }
  ]
}

NOTES: "C2"–"B5". Bass octaves 2–3. Melody 4–5.
DRUMS: "kick" (beats 1,3)  "snare" (2,4)  "hat" (subdivisions)  "clap"  "openhat"
WAVE: "sine" "triangle" "square" "sawtooth"
PAN: kick/bass/snare=0.0 · hat=0.3–0.5 · melody spread -0.3 to 0.3 · pads wide ±0.6
ATTACK: 0.001 pluck · 0.02 normal · 0.1–0.3 pad swell
RELEASE: 0.05 staccato · 0.15 normal · 0.5–1.0 sustained
REVERB: 0.15 room · 0.3 hall · 0.6 ambient"#;

// ── Song section planning ─────────────────────────────────────────────────────

pub struct SectionPlan {
    pub name: String,
    pub bars: u32,
    pub role: String,
}

/// Plans a full song as a sequence of *unique* sections — every slot gets its own
/// LLM call, so no audio buffer is ever reused. Scales section count and bar length
/// to approximately fill `target_secs`.
pub fn plan_song(target_secs: u32, bpm_hint: f32) -> Vec<SectionPlan> {
    let bar_secs = (60.0 / bpm_hint) * 4.0;

    // Aim for sections of ~28 s each; clamp total section count to 5–12
    let n = ((target_secs as f32 / 28.0).round() as usize).clamp(5, 12);

    // Bars per section: round to nearest 4-bar phrase, cap at 16 for LLM token budget
    let raw = (target_secs as f32 / n as f32 / bar_secs).round() as u32;
    let section_bars = ((raw + 2) / 4 * 4).clamp(4, 16);

    // Section library — ordered for natural song arc; pick first `n` entries
    let lib: &[(&str, &str)] = &[
        ("intro",
         "INTRO — Sparse and atmospheric. Bass and very light hi-hat only. NO melody. \
          Low energy; slowly build anticipation. Keep the arrangement intentionally empty."),
        ("verse_1",
         "VERSE 1 — Full arrangement. Bass groove + lead melody + full drums (kick, snare, hat). \
          Medium energy. Establish a clear, memorable melodic phrase. \
          Use a consistent key throughout."),
        ("build",
         "BUILD — Rising energy. Layer a counter-melody or arpeggio pad on top of the verse groove. \
          Add extra percussion hits on off-beats. Tension building toward the drop."),
        ("chorus_1",
         "CHORUS 1 — The hook. Maximum energy. Every layer at once: bass, lead melody, pads, \
          full drums + clap. Bright, uplifting, harmonically rich. Emotional peak."),
        ("verse_2",
         "VERSE 2 — Same key and BPM as verse 1, but a COMPLETELY DIFFERENT melodic phrase \
          and a varied rhythmic pattern. The song progresses; do not repeat verse 1."),
        ("chorus_2",
         "CHORUS 2 — Hook returns. Same feel as chorus 1 but add extra percussion hits or \
          a higher melody line to build more intensity than chorus 1."),
        ("bridge",
         "BRIDGE — Contrast and tension. Strip most layers. Change the rhythmic feel, or drop \
          the kick entirely. Sparse and unexpected — create space before the finale."),
        ("drop",
         "DROP — Minimal. Bass-heavy hypnotic groove with almost no melody. Atmospheric pads only. \
          Stripped-back tension builder before the final rise."),
        ("chorus_3",
         "FINAL CHORUS — Biggest moment of the entire track. Maximum intensity: all instruments \
          at full power. Add extra layers to make it sound huge and triumphant."),
        ("outro_1",
         "OUTRO 1 — Begin winding down. Remove the lead melody. Only bass + light drums. \
          Energy decreasing gradually."),
        ("outro_2",
         "OUTRO 2 — Very sparse. Remove drums entirely. Only bass and a soft pad fading away."),
        ("end",
         "END — Final resolution. One or two very quiet sustained notes only. Song ending."),
    ];

    lib[..n]
        .iter()
        .map(|(name, role)| SectionPlan {
            name: name.to_string(),
            bars: section_bars,
            role: role.to_string(),
        })
        .collect()
}

// ── Request / response types ──────────────────────────────────────────────────

pub struct LlmRequest {
    pub prompt: String,
    pub model: String,
    pub api_key: String,
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
