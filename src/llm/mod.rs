pub mod cerebras;
pub mod cohere;

// ── Base JSON format used by every section call ───────────────────────────────

pub const FORMAT_RULES: &str = r#"You are both composer AND sound engineer. Every musical and sonic parameter is yours to decide.
Think like a professional music producer — create a FULL, LAYERED arrangement.

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

━━ CRITICAL COMPOSITION RULES ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
1. HARMONY: Pick a KEY (e.g. C minor, F major) and stay in it. Every melodic note
   MUST belong to that scale. Use chord tones on strong beats.
2. LAYERS: You MUST include at minimum: bass line + lead melody + drums (kick+snare+hat).
   Great pieces also include: pad/chords + counter-melody or arpeggios.
3. RHYTHM: Vary rhythmic patterns. Use syncopation, offbeat accents, and ghost notes.
   Never just place notes on every beat — leave breathing room.
4. MELODY: Create a singable, memorable phrase. Use stepwise motion with occasional leaps.
   Repeat the core motif with subtle variation to create hooks.
5. BASS: Root notes on downbeats, passing tones on upbeats. Octave jumps add energy.
   Bass should complement the kick pattern, not collide with it.
6. DYNAMICS: Vary gain across events for musical expression. Downbeats louder,
   ghost notes quieter. Build and release tension across bars.

━━ TIMING ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
4/4 time. t=0 = bar 1 beat 1.  t=4 = bar 2 beat 1.  Beat = 1 unit.
Fill ALL requested bars with events. Use subdivisions: 0.25=16th, 0.5=8th, 1=quarter.
Create patterns that evolve — bars 3-4 should develop or vary from bars 1-2.

━━ NOTES ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Pitches: "C2"–"B5"  (e.g. "C4", "Eb3", "F#5", "Bb4")
Bass lines: octaves 2–3 · Melody / lead: 4–5 · Pads: 3–4
Use chord voicings: e.g. for Cm, stack C3+Eb3+G3 as simultaneous pad events.

━━ DRUMS ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
"kick"    — primary thump (often beats 1, 3; can vary per genre)
"snare"   — crack (beats 2, 4 in most styles)
"hat"     — closed hi-hat for tight subdivisions (8ths, 16ths)
"openhat" — open hi-hat for sustained shimmer (end of phrases)
"clap"    — layered with snare or on offbeats for accent
"rim"     — rimshot click for subtle percussive accents
"ride"    — ride cymbal for smooth sustained groove
"crash"   — cymbal crash on section transitions / downbeat 1
"tom"     — mid tom fill  "tomhi" — high tom  "tomlo" — floor tom

Drum tips: Add ghost notes (hat at gain 0.20-0.30), vary velocity on hats for
a human feel, place openhat on the "and" of beat 4 before a new phrase.

━━ WAVE (melodic only) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
"sine"      — smooth, pure, warm  (best for: sub-bass, pads, mellow leads)
"triangle"  — soft, gentle, slightly bright (best for: pads, flute-like leads)
"square"    — hollow, woody, retro (best for: lo-fi organ, chiptune, bass)
"sawtooth"  — bright, buzzy, full (best for: synth leads, brass, rich bass)

Match the wave to the role: bass=sine/sawtooth, lead=saw/triangle, pad=sine/triangle.

━━ GAIN ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Kick 0.80–0.90 · Snare 0.65–0.75 · Hat 0.30–0.50 · Clap 0.55–0.65
Bass 0.55–0.70 · Lead melody 0.45–0.60 · Pad/chord 0.20–0.35
Ghost notes: 0.15–0.25 · Accents: +0.10 above normal

━━ PAN ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
-1.0 hard-left · 0.0 center · 1.0 hard-right
Kick, bass, snare → center (0.0)
Hi-hat → slight right (0.3–0.5) · Open hat → center-right (0.2)
Clap → slight left (-0.2) · Rim → slight right (0.15)
Lead melody → off-center (-0.15 to 0.25)
Pad chord voices → spread wide: root=center, 3rd=-0.5, 5th=+0.5

━━ ATTACK & RELEASE (melodic only, in seconds) ━━━━━━━━━━━━
attack:  0.001 pluck · 0.01 keys · 0.03 normal · 0.08–0.3 pad swell
release: 0.05 staccato · 0.15 normal · 0.3–0.8 sustained · 1.0+ ambient pad
Use LONGER release for pads (0.5–1.0) to create a bed of sound.
Use SHORTER attack for percussive leads (0.001–0.005).

━━ REVERB (whole pattern) ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
0.0 completely dry · 0.12 tight room · 0.25 medium hall
0.4 large hall · 0.55 cathedral · 0.75 huge ambient wash
Match to genre: techno 0.10–0.20 · lo-fi 0.25–0.40 · ambient 0.50–0.75 · jazz 0.20–0.35"#;

// ── Single-pattern prompt (loop mode) ────────────────────────────────────────

pub const STRUDEL_SYSTEM_PROMPT: &str = r#"You are a professional music composer and sound engineer for a CLI synthesizer.
Generate a complete, musically satisfying piece that sounds great.

CRITICAL REQUIREMENTS:
1. Use 8–16 bars. The piece plays once — create a beginning, development, and resolution.
2. Pick a KEY (e.g. C minor, Eb major) and EVERY melodic note must belong to that scale.
3. Layer AT LEAST 4 elements: bass + lead melody + drums (kick+snare+hat) + pad/chords.
4. Use panning, reverb, and ADSR to shape a professional, spacious mix.
5. Create MEMORABLE melodies — use motifs, repetition with variation, and singable phrases.
6. Vary dynamics: ghost notes on hats, accented beats, build tension over bars.
7. Bars 1-4 establish the groove; bars 5-8+ develop and vary it. Never copy-paste.

Output ONLY a JSON object — no markdown fences, no extra text. Use the FULL schema:
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

NOTES: "C2"–"B5". Bass octaves 2–3. Lead melody 4–5. Pads/chords 3–4.
For chords, stack multiple simultaneous notes (e.g. C3+Eb3+G3 for Cm chord).
DRUMS: "kick" "snare" "hat" "openhat" "clap" "rim" "ride" "crash" "tom" "tomhi" "tomlo"
WAVE: "sine" (warm) "triangle" (soft) "square" (hollow) "sawtooth" (bright, buzzy)
PAN: kick/bass/snare=0.0 · hat=0.3–0.5 · melody=-0.15 to 0.25 · pads spread ±0.5
ATTACK: 0.001 pluck · 0.01 keys · 0.03 normal · 0.1–0.3 pad swell
RELEASE: 0.05 staccato · 0.15 normal · 0.4–0.8 sustained · 1.0+ ambient
REVERB: 0.12 room · 0.25 hall · 0.40 large hall · 0.60 ambient
GAIN: kick 0.80–0.90 · snare 0.65–0.75 · hat 0.30–0.50 · bass 0.55–0.70
  lead 0.45–0.60 · pad 0.20–0.35 · ghost notes 0.15–0.25"#;

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
         "INTRO — Atmospheric and evocative. Start with a sustained pad chord (long attack 0.2s, \
          release 1.0s) plus a gentle ride cymbal pattern. Maybe one bass note per bar. \
          NO full drums, NO melody yet. Low energy; create a mood. Use reverb 0.4–0.6. \
          2–3 layers maximum."),
        ("verse_1",
         "VERSE 1 — The groove begins. Full bass line with a walking/syncopated pattern. \
          Lead melody enters: create a MEMORABLE, SINGABLE phrase of 4-8 notes that repeats \
          with subtle variation. Full drums: kick on 1+3, snare on 2+4, hat on 8ths with \
          velocity variation (ghost notes at gain 0.20). Add a pad underneath for warmth. \
          Stay in ONE key. Medium energy."),
        ("build",
         "BUILD — Rising tension. Keep the bass and drums from verse. Layer an ARPEGGIO pattern \
          (16th notes cycling through chord tones, e.g. C4-Eb4-G4-Eb4 repeating). Add open hi-hat \
          on the 'and' of beat 4. Increase hat density to 16ths. Add a rim shot on offbeats. \
          Everything pushes toward the chorus."),
        ("chorus_1",
         "CHORUS 1 — The hook. MAXIMUM energy. ALL layers active: driving bass (octave jumps for \
          energy), bright lead melody (use sawtooth wave, slightly higher register than verse), \
          full drums with clap layered on snare, crash cymbal on beat 1. Add a wide pad chord \
          (3 notes panned -0.5, 0.0, +0.5). This is the emotional peak — make it feel BIG."),
        ("verse_2",
         "VERSE 2 — Same key and BPM. DIFFERENT melodic phrase from verse 1 — develop the theme, \
          don't repeat it. Vary the bass rhythm. Change hat pattern slightly. Add a counter-melody \
          or harmony line panned opposite to the lead. The song PROGRESSES."),
        ("chorus_2",
         "CHORUS 2 — Hook returns with MORE intensity. Same feel as chorus 1 but: add tom fills \
          at bar transitions, push the melody an octave higher for key moments, add a second \
          harmony lead panned opposite. Cymbal crash on beat 1. Fuller than chorus 1."),
        ("bridge",
         "BRIDGE — Contrast and surprise. DROP the kick entirely. Change to half-time feel or \
          remove the snare. Use only pads + a sparse melodic fragment + ride cymbal. \
          Different chord progression or modal shift. Create SPACE and TENSION. \
          This makes the return of the chorus feel earned."),
        ("drop",
         "DROP — The kick returns with FORCE. Heavy, hypnotic bass pattern (simple but powerful). \
          Minimal melody — only a short riff or stab. Atmospheric pad drone underneath. \
          Build anticipation: gradually add hat density, introduce open hats, rise in intensity \
          across bars. Reverb 0.15–0.25 for a tight, focused sound."),
        ("chorus_3",
         "FINAL CHORUS — The BIGGEST moment. Everything from chorus 2 PLUS: extra percussion \
          (rim, clap, crash), bass at full energy with fills, melody at peak register. \
          Add a triumphant counter-melody. This should feel climactic and euphoric. \
          Every instrument at full power. Crash cymbal on beat 1."),
        ("outro_1",
         "OUTRO 1 — Begin the descent. Remove the lead melody. Bass simplifies to root notes. \
          Drums thin out: remove clap, reduce hat density to quarter notes. \
          Keep the pad for continuity. Energy decreasing bar by bar. \
          Reduce gain on remaining elements by 0.05–0.10 compared to chorus."),
        ("outro_2",
         "OUTRO 2 — Nearly gone. Remove all drums. Only a sustained bass note and a soft pad \
          chord with long release (1.0s+). Maybe one final melodic motif callback, very quiet. \
          Reverb 0.5+ for a wash of sound. 2 layers maximum."),
        ("end",
         "END — Final resolution. A single sustained chord (root + fifth) with very long \
          release (1.5s+), gain 0.15–0.25, fading to silence. The song ends with breath."),
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
