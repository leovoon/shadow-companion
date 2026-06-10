# 🦭 Shadow Companion

TTS echo for language shadowing with [Handy](https://github.com/cjpais/Handy).

## How it works

1. You read an article aloud
2. Handy transcribes your speech → saves to history database
3. This companion watches Handy's DB → TTS speaks your words back
4. You shadow the native pronunciation, repeat

Supports two TTS providers:
- **Kokoro** — Built-in voices, adjustable speed, fast startup
- **NeuTTS Air** — Voice cloning from your own reference audio, streaming playback

## Setup

```bash
cd ~/shadow-companion
python3 -m venv .venv
source .venv/bin/activate

# Install dependencies (Kokoro only)
pip install pykokoro pyperclip sounddevice numpy Pillow

# For NeuTTS support, also install:
pip install neutts llama-cpp-python onnxruntime

# Download English language model (Kokoro)
python -m spacy download en_core_web_sm
```

## Usage

### Start the server (background)

```bash
python shadow.py serve
python shadow.py serve --provider neutts   # Use NeuTTS voice cloning
```

### Control commands

```bash
python shadow.py status                     # Check if running
python shadow.py stop                       # Stop server
python shadow.py restart                    # Restart server
python shadow.py set-voice am_adam          # Change voice (Kokoro, hot-reloads)
python shadow.py set-speed 0.85             # Change speed (Kokoro only)
python shadow.py set-provider kokoro        # Switch TTS engine (requires restart)
python shadow.py set-provider neutts        # Switch to NeuTTS (requires restart)
```

### Or run in foreground

```bash
python shadow.py                            # Default: kokoro, am_michael, 1.0x speed
python shadow.py --voice am_adam
python shadow.py --speed 0.85
python shadow.py --provider neutts
```

## NeuTTS Voice Cloning

NeuTTS Air clones a voice from a short reference recording. This lets you practice shadowing with a voice similar to your own — or a native speaker's.

### Setup

```bash
python shadow.py setup-voice
```

This records ~15 seconds of speech and saves:
- `~/.shadow-companion/my-voice.wav` — reference audio
- `~/.shadow-companion/my-voice.txt` — reference transcript

### Using a native speaker's voice

Replace the reference files with a native speaker's recording (10-15s of clean speech):

```bash
cp native-speaker.wav ~/.shadow-companion/my-voice.wav
echo "The transcript of that recording" > ~/.shadow-companion/my-voice.txt
```

Then restart: `python shadow.py restart`

### Notes

- NeuTTS uses streaming playback (`infer_stream()`) for low-latency audio — chunks play as they're generated
- Speed control is not available with NeuTTS
- Provider changes require a server restart
- Only GGUF backbones are supported for streaming (Q8 by default)

## Daily Tracking

A macOS menubar meter built with [Perry](https://github.com/PerryTS/perry) that shows your daily shadowing time. Completely optional — the core shadow companion works without it.

### What it tracks

- **TTS playback duration** — how long you spent listening/shadowing (primary metric)
- **STT recording duration** — how long you spoke into Handy (secondary, shown in `verify`)
- Current local day only (not lifetime)
- Writes `~/.shadow-companion/daily-progress.json` for the menubar app to read
- TTS play log stored in `~/.shadow-companion/tts-play-log.json`

### CLI commands

```bash
python shadow.py progress              # Compute and print today's progress
python shadow.py verify                # Detailed audit: TTS time, STT time, per-recording breakdown
python shadow.py set-daily-target 30   # Set daily target in minutes (default: 60)
```

### Menubar app

A compact Perry-native tray icon in your macOS menu bar:

- 🔋 **Battery mode** — 5-slice visual (click to toggle)
- 📝 **Text mode** — shows minutes as `X/Y`
- **Tooltip** — hover for exact progress
- **Context menu** — click for config, progress file, toggle mode, quit
- **No Dock icon** — lives only in the menubar

#### Setup

```bash
# 1. Generate icon PNGs
python generate_icons.py

# 2. Compute initial progress
python shadow.py progress

# 3. Compile the Perry app (requires Perry)
perry compile src/main.ts -o dist/shadow-meter

# 4. Run it
./dist/shadow-meter
```

Or use the build script:

```bash
./build.sh
open "dist/Shadow Meter.app"
```

#### How it updates

The server logs TTS playback duration after each utterance and writes `daily-progress.json` whenever it processes a new Handy recording. The menubar app polls this file every 15 seconds.

#### Verifying accuracy

```bash
python shadow.py verify
```

Shows TTS playback time, STT recording time, every recording counted, its WAV duration, whether the file still exists, totals, and cross-checks against `daily-progress.json`.

### Daily target

Default is 60 minutes. Change it:

```bash
python shadow.py set-daily-target 45   # 45 minutes
```

Progress formula: `tts_playback_duration_today / daily_target_duration`, clamped 0–1. Battery slices = `ceil(progress × 5)`.

## Raycast Extension

Control Shadow Companion from Raycast:

```bash
cd ~/shadow-companion/raycast-extension
npm install
npm run dev
```

Four commands available:
- **Control Server** — Start/stop/restart + see status, provider, and daily progress
- **Switch Voice** — Pick from all Kokoro voices (shows NeuTTS info when that provider is active)
- **Switch Provider** — Switch between Kokoro and NeuTTS (with restart action)
- **Adjust Speed** — Set speech speed, 0.5x–2.0x (Kokoro only; shows "not available" for NeuTTS)

## Available Kokoro voices

| Male | Female |
|------|--------|
| **am_michael** (default) | af_heart |
| am_adam | af_nicole |
| am_eric | af_sarah |
| am_liam | af_bella |
| am_onyx | af_river |
| am_puck | af_sky |

## Troubleshooting

**CoreML slow on M2:** Use `--provider cpu` (default) — it's actually faster (~4-5× realtime) because CoreML only supports ~45% of Kokoro's ONNX nodes, causing graph partitioning overhead.

**NeuTTS audio choppy:** Fixed — streaming now uses `sounddevice.OutputStream` with a queue-based callback for gapless chunk playback. If issues persist, check that `llama-cpp-python` and `onnxruntime` are up to date.

**"Could not find Handy's history.db":** Make sure Handy is installed and has been used at least once. Or specify the path: `python shadow.py --db /path/to/history.db`

**Server not responding:** Check the log at `~/.shadow-companion/server.log`

**Menubar shows stale data:** Run `python shadow.py progress` to recompute, or just wait — the server updates on every new recording.

**NeuTTS reference audio missing:** Run `python shadow.py setup-voice` to record your reference audio. Or place a WAV file at `~/.shadow-companion/my-voice.wav` with a matching `.txt` transcript.
