# 🦭 Shadow Companion

TTS echo for language shadowing with [Handy](https://github.com/cjpais/Handy).

## How it works

1. You read an article aloud
2. Handy transcribes your speech → saves to history database
3. This companion watches Handy's DB → Kokoro-82M speaks your words back
4. You shadow the native pronunciation, repeat

## Setup

```bash
cd ~/shadow-companion
python3 -m venv .venv
source .venv/bin/activate

# Install dependencies
pip install pykokoro pyperclip sounddevice numpy Pillow

# Download English language model
python -m spacy download en_core_web_sm
```

## Usage

### Start the server (background)

```bash
python shadow.py serve
```

### Control commands

```bash
python shadow.py status        # Check if running
python shadow.py stop          # Stop server
python shadow.py restart       # Restart server
python shadow.py set-voice am_adam   # Change voice (hot-reloads)
python shadow.py set-speed 0.85      # Change speed
```

### Or run in foreground

```bash
python shadow.py               # Default: am_michael, 1.0x speed
python shadow.py --voice am_adam
python shadow.py --speed 0.85
```

## Daily Tracking (Optional)

A macOS menubar meter built with [Perry](https://github.com/PerryTS/perry) that shows your daily Handy STT recording time. Completely optional — the core shadow companion works without it.

### What it tracks

- **STT recording duration** for the current local day only
- Not lifetime progress. Not TTS playback duration.
- Reads WAV headers from Handy's recordings directory (no audio decoding)
- Writes `~/.shadow-companion/daily-progress.json` for the menubar app to read

### CLI commands

```bash
python shadow.py progress              # Compute and print today's progress
python shadow.py verify                # Detailed audit: every recording, duration, cross-check
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

#### How it updates

The server automatically writes `daily-progress.json` whenever it processes a new Handy recording. The menubar app polls this file every 15 seconds — negligible battery impact (one `readFileSync` of a ~100-byte file).

#### Verifying accuracy

```bash
python shadow.py verify
```

Shows every recording counted, its WAV duration, whether the file still exists, totals, and cross-checks against `daily-progress.json`.

### Daily target

Default is 60 minutes. Change it:

```bash
python shadow.py set-daily-target 45   # 45 minutes
```

Progress formula: `actual_stt_duration_today / daily_target_duration`, clamped 0–1. Battery slices = `ceil(progress × 5)`.

## Raycast Extension

Control Shadow Companion from Raycast:

```bash
cd ~/shadow-companion/raycast-extension
npm install
npm run dev
```

Three commands available:
- **Control Server** — Start/stop/restart + see status
- **Switch Voice** — Pick from all English voices
- **Adjust Speed** — Set speech speed (0.7x – 1.3x)

## Available English voices

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

**"Could not find Handy's history.db":** Make sure Handy is installed and has been used at least once. Or specify the path: `python shadow.py --db /path/to/history.db`

**Server not responding:** Check the log at `~/.shadow-companion/server.log`

**Menubar shows stale data:** Run `python shadow.py progress` to recompute, or just wait — the server updates on every new recording.
