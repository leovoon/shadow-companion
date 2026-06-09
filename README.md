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
pip install pykokoro pyperclip sounddevice numpy

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

## Latency on M2

With CPU provider, Kokoro runs at **4-5× realtime** after warmup. Typical sentence (2-3s of audio) generates in ~0.5-0.8s. The companion uses multiple optimizations:

- **kqueue file watching** — instant DB change detection instead of polling
- **Segment streaming** — first sentence plays while subsequent sentences are still generating
- **Eager pipeline + pre-warm** — ONNX session ready before first speech event
- **Fast path for short input** — single-sentence goes through simpler pipe.run()

Total loop:
- You speak → Handy transcribes (~1-3s)
- DB change detected → Kokoro generates first segment (~0.3-0.5s)
- Audio plays → remaining segments stream → you shadow

**CoreML is slower** on M2 because only ~45% of Kokoro's ONNX nodes are supported, causing graph partitioning overhead. CPU wins.

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
