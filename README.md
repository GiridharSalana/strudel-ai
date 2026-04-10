# Strudel-Ai

**AI-powered music generator that runs entirely in your terminal.**

Describe the music you want in plain English. An LLM composes a layered pattern — drums, bass, melody, pads — and your computer plays it through your speakers. No browser, no GUI, just a CLI and sound.

```
Strudel-Ai "a dark techno groove with acid bass"
```

https://github.com/user-attachments/assets/placeholder-demo.gif

---

## How it works

```
prompt → Cerebras / Cohere LLM → JSON pattern → Rust synthesizer → aplay → speakers
```

1. Your prompt is sent to an LLM (Cerebras or Cohere) with a system prompt that instructs it to compose music as a structured JSON event list.
2. The Rust synthesizer renders the events into a WAV buffer using oscillators (sine, square, sawtooth, triangle) and drum synthesis (pitch-swept kick, noise-burst snare/hat/clap).
3. The WAV is piped to `aplay` and played back in a loop.

---

## Features

- **Entirely CLI** — no browser, no Electron, no runtime dependencies
- **Two LLM backends** — Cerebras (fast) or Cohere
- **Built-in synthesizer** — sine, square, sawtooth, triangle oscillators + kick, snare, hat, clap drums
- **Loops seamlessly** — fades at loop boundaries, plays 8× by default
- **Export** — save as `.wav` or raw `.json` pattern

---

## Install

### Prerequisites

- Rust (install via [rustup.rs](https://rustup.rs))
- `aplay` — part of `alsa-utils` (pre-installed on most Linux distros)
- A Cerebras or Cohere API key

### Build

```bash
git clone https://github.com/GiridharSalana/strudel-ai
cd strudel-ai
cargo build --release
```

The binary will be at `target/release/Strudel-Ai`.

---

## Usage

```bash
# Set your API key (add to ~/.bashrc or ~/.config/fish/config.fish)
export CEREBRAS_API_KEY="csk-..."

# Generate and play music
Strudel-Ai "a groovy lo-fi hip hop beat with jazzy piano"

# Use Cohere instead
Strudel-Ai "ambient drone with evolving pads" --provider cohere --api-key co_...

# Print the generated pattern JSON without playing
Strudel-Ai "upbeat jazz trio" --print-code --no-play

# Export to WAV
Strudel-Ai "cinematic orchestral swell" --output track.wav

# Save pattern JSON (reuse or inspect later)
Strudel-Ai "acid techno loop" --output pattern.json

# Use a larger model for richer compositions
Strudel-Ai "complex polyrhythmic fusion" --model qwen-3-235b-a22b-instruct-2507
```

### All options

```
Usage: Strudel-Ai [OPTIONS] <PROMPT>

Arguments:
  <PROMPT>  Describe the music you want to generate

Options:
  -p, --provider <PROVIDER>  LLM provider [default: cerebras] [possible values: cerebras, cohere]
  -k, --api-key <KEY>        API key (or set CEREBRAS_API_KEY / COHERE_API_KEY env var)
  -m, --model <MODEL>        Model override (see providers below)
      --no-play              Generate pattern but don't play audio
  -o, --output <FILE>        Save output (.wav for audio, anything else for JSON)
      --print-code           Print the generated JSON pattern to stdout
  -h, --help                 Print help
  -V, --version              Print version
```

---

## Providers & Models

| Provider  | Default model          | Other models                         | Env var            |
|-----------|------------------------|--------------------------------------|--------------------|
| Cerebras  | `llama3.1-8b`          | `qwen-3-235b-a22b-instruct-2507`     | `CEREBRAS_API_KEY` |
| Cohere    | `command-a-03-2025`    | `command-r-plus`                     | `COHERE_API_KEY`   |

---

## Pattern format

The LLM outputs a JSON object. You can inspect it with `--print-code`, edit it, save it, and replay it however you like.

```json
{
  "bpm": 90,
  "bars": 4,
  "events": [
    { "t": 0.0, "dur": 1.0, "note": "kick",  "gain": 0.85 },
    { "t": 1.0, "dur": 1.0, "note": "snare", "gain": 0.70 },
    { "t": 0.0, "dur": 0.5, "note": "hat",   "gain": 0.45 },
    { "t": 0.0, "dur": 2.0, "note": "C2",    "wave": "sawtooth", "gain": 0.60 },
    { "t": 0.0, "dur": 1.0, "note": "C4",    "wave": "sine",     "gain": 0.50 }
  ]
}
```

| Field  | Description |
|--------|-------------|
| `t`    | Start time in beats (0 = bar 1 beat 1, 4 = bar 2 beat 1) |
| `dur`  | Duration in beats (0.25 = 16th, 0.5 = 8th, 1 = quarter) |
| `note` | Note name (`C4`, `Eb3`, `F#5`) or drum (`kick`, `snare`, `hat`, `clap`) |
| `wave` | Waveform for melodic notes: `sine`, `square`, `sawtooth`, `triangle` |
| `gain` | Volume 0.0–1.0 |

---

## License

MIT
