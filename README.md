# giribeat

> AI-powered music composer for the terminal. Describe what you want to hear — giribeat composes it, synthesizes it, and plays it through your speakers.

```
giribeat "dark techno groove with acid bass"
giribeat "5 minutes of lo-fi hip hop" --duration 5m
```

No browser. No GUI. No runtime dependencies. Just a prompt and sound.

---

## Features

- **Plain-English prompts** — describe any style, mood, or instrumentation
- **Full songs** — multi-section compositions (intro → verse → chorus → bridge → outro) timed to a target duration
- **Built-in synthesizer** — anti-aliased oscillators via [fundsp](https://github.com/SamiPerttu/fundsp), Schroeder reverb, pitch-swept kick, noise-body snare
- **Two LLM backends** — [Cerebras](https://cerebras.ai) (fast, free tier) or [Cohere](https://cohere.com)
- **Export** — save as `.wav` or inspect the raw pattern `.json`

---

## Installation

**Requirements**

- [Rust](https://rustup.rs) (stable)
- `aplay` — ships with `alsa-utils`, pre-installed on most Linux distributions
- A [Cerebras](https://cloud.cerebras.ai) or [Cohere](https://cohere.com) API key

```bash
git clone https://github.com/GiridharSalana/giribeat
cd giribeat
cargo build --release
# binary is at target/release/giribeat

# Optional: install to PATH
cargo install --path .
```

---

## Usage

```bash
export CEREBRAS_API_KEY="csk-..."   # or COHERE_API_KEY

# Compose and play a looping pattern
giribeat "melancholic jazz piano trio"

# Generate a full 5-minute song
giribeat "cinematic orchestral buildup" --duration 5m

# Use duration directly in the prompt
giribeat "3 minutes of dark ambient drone"

# Export to WAV
giribeat "upbeat indie pop" --output song.wav

# Inspect the generated pattern without playing
giribeat "acid techno" --print-code --no-play

# Use Cohere as the LLM backend
giribeat "bossa nova" --provider cohere --api-key co_...

# Use a larger model for richer compositions
giribeat "complex polyrhythmic fusion" --model qwen-3-235b-a22b-instruct-2507
```

### Options

```
Usage: giribeat [OPTIONS] <PROMPT>

Arguments:
  <PROMPT>   Describe the music you want — freely, in plain English

Options:
  -d, --duration <DURATION>   Target song length. Enables song mode.
                              Formats: 5m · 3min · 2:30 · 300 (seconds)
  -p, --provider <PROVIDER>   LLM backend [default: cerebras]
                              [possible values: cerebras, cohere]
  -k, --api-key <KEY>         API key (overrides env var)
  -m, --model <MODEL>         Model override (see table below)
  -o, --output <FILE>         Save output (.wav = audio, other = JSON)
      --no-play               Compose but don't play audio
      --print-code            Print the pattern JSON to stdout
  -h, --help                  Print help
  -V, --version               Print version
```

### LLM Providers

| Provider  | Env var            | Default model                 | Fast alternative                   |
|-----------|--------------------|-------------------------------|-------------------------------------|
| Cerebras  | `CEREBRAS_API_KEY` | `llama3.1-8b`                | `qwen-3-235b-a22b-instruct-2507`   |
| Cohere    | `COHERE_API_KEY`   | `command-a-03-2025`          | `command-r-plus`                    |

You can also set `GIRIBEAT_API_KEY` as a provider-agnostic fallback.

---

## How it works

```
prompt ──► LLM (Cerebras / Cohere)
              │
              ▼
         JSON pattern
         { bpm, bars, events[] }
              │
              ▼
         Rust synthesizer
         ├─ fundsp oscillators (anti-aliased sine/saw/square/triangle)
         ├─ drum synthesis    (pitch-swept kick, noise-body snare/hat/clap)
         └─ Schroeder reverb on melodic layer
              │
              ▼
         WAV buffer ──► aplay ──► speakers
```

**Song mode** makes one LLM call per section — intro, verse A, verse B, chorus, bridge, outro — locking BPM across all sections. The arrangement algorithm fills the requested duration by repeating verse/chorus pairs, then closes with bridge → chorus → outro.

---

## Pattern format

The LLM outputs — and you can inspect, edit, or replay — a simple JSON schema:

```json
{
  "bpm": 90,
  "bars": 4,
  "events": [
    { "t": 0,   "dur": 1,   "note": "kick",  "gain": 0.85 },
    { "t": 1,   "dur": 1,   "note": "snare", "gain": 0.70 },
    { "t": 0,   "dur": 0.5, "note": "hat",   "gain": 0.45 },
    { "t": 0,   "dur": 2,   "note": "C2",    "wave": "sawtooth", "gain": 0.60 },
    { "t": 0,   "dur": 1,   "note": "C4",    "wave": "sine",     "gain": 0.50 }
  ]
}
```

| Field  | Type   | Description |
|--------|--------|-------------|
| `t`    | float  | Start time in beats. `0` = bar 1 beat 1, `4` = bar 2 beat 1. |
| `dur`  | float  | Duration in beats. `0.25` = 16th, `0.5` = 8th, `1` = quarter. |
| `note` | string | Pitch (`C4`, `Eb3`, `F#5`) or drum (`kick`, `snare`, `hat`, `clap`). |
| `wave` | string | Waveform for pitched notes: `sine` `square` `sawtooth` `triangle`. |
| `gain` | float  | Amplitude 0.0–1.0. |

---

## License

MIT © [Giridhar Salana](https://github.com/GiridharSalana)
