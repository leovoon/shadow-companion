#!/usr/bin/env python3
"""
Shadow Companion — watches Handy's transcription history database,
speaks new entries back with TTS so you can shadow native intonation.

Supports two TTS providers:
  kokoro  — Kokoro TTS (built-in voices, adjustable speed)
  neutts  — NeuTTS Air (voice cloning, requires reference audio)

Usage:
    python shadow.py [--voice VOICE] [--speed SPEED] [--provider kokoro|neutts]

Server mode (for Raycast/CLI control):
    python shadow.py serve                    # start as background server
    python shadow.py stop                     # stop running server
    python shadow.py status                   # check if server is running
    python shadow.py set-voice <voice>        # change voice (hot-reloads, kokoro only)
    python shadow.py set-speed <speed>        # change speed (kokoro only)
    python shadow.py set-provider <provider>  # change TTS engine (requires restart)
    python shadow.py setup-voice              # record reference audio for NeuTTS
"""

import argparse
import json
import os
import signal
import sqlite3
import subprocess
import sys
import threading
import time
from pathlib import Path

# ── voices ────────────────────────────────────────────────────────
VOICE_LIST = [
    "af_heart", "af_nicole", "af_sarah", "af_bella", "af_river",
    "af_sky", "af_nova", "af_alloy", "af_aoede", "af_kore",
    "am_michael", "am_adam", "am_eric", "am_liam", "am_onyx",
    "am_puck", "am_echo", "am_fenrir",
]

POLL_S = 0.1  # Fallback poll interval (kqueue used when available)
IDLE_TIMEOUT_S = 600  # Kill TTS worker after this many seconds idle (10 min)


# Server state file
STATE_DIR = Path.home() / ".shadow-companion"
STATE_FILE = STATE_DIR / "state.json"
PID_FILE = STATE_DIR / "server.pid"
DAILY_PROGRESS_FILE = STATE_DIR / "daily-progress.json"
TTS_PLAY_LOG = STATE_DIR / "tts-play-log.json"

# NeuTTS reference voice defaults
REF_VOICE_WAV = STATE_DIR / "my-voice.wav"
REF_VOICE_TXT = STATE_DIR / "my-voice.txt"

# Default daily target in seconds (60 minutes)
DEFAULT_DAILY_TARGET_S = 3600


# ── state management ──────────────────────────────────────────────

def load_state() -> dict:
    if STATE_FILE.exists():
        try:
            return json.loads(STATE_FILE.read_text())
        except Exception:
            pass
    return {"voice": "am_michael", "speed": 1.0, "provider": "kokoro", "running": False, "daily_target_s": DEFAULT_DAILY_TARGET_S}


def save_state(state: dict):
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    STATE_FILE.write_text(json.dumps(state, indent=2))


# ── daily progress ────────────────────────────────────────────────

def get_handy_recordings_dir() -> Path | None:
    """Find Handy's recordings directory on macOS."""
    base = Path.home() / "Library" / "Application Support" / "com.pais.handy"
    rec = base / "recordings"
    if rec.exists():
        return rec
    return None


def wav_duration(path: Path) -> float:
    """Read duration from WAV header using stdlib wave (no audio decoding)."""
    import wave
    try:
        with wave.open(str(path), "rb") as wf:
            frames = wf.getnframes()
            rate = wf.getframerate()
            return frames / rate if rate > 0 else 0.0
    except Exception:
        return 0.0


def log_tts_play(duration_s: float):
    """Append TTS playback duration to today's log."""
    from datetime import date

    STATE_DIR.mkdir(parents=True, exist_ok=True)
    log = {}
    if TTS_PLAY_LOG.exists():
        try:
            log = json.loads(TTS_PLAY_LOG.read_text())
        except (json.JSONDecodeError, ValueError):
            log = {}

    today = date.today().isoformat()
    log[today] = round(log.get(today, 0.0) + duration_s, 1)
    TTS_PLAY_LOG.write_text(json.dumps(log, indent=2))


def compute_daily_tts_duration() -> float:
    """Compute total TTS playback duration for today from the play log."""
    from datetime import date

    if not TTS_PLAY_LOG.exists():
        return 0.0
    try:
        log = json.loads(TTS_PLAY_LOG.read_text())
    except (json.JSONDecodeError, ValueError):
        return 0.0

    today = date.today().isoformat()
    return log.get(today, 0.0)


def compute_daily_stt_duration(db_path: Path) -> float:
    """Compute total STT recording duration for today (local timezone) in seconds."""
    import wave as _wave
    from datetime import date, datetime, timezone

    today = date.today()
    # Naive datetime = local midnight. .timestamp() converts to UTC epoch correctly.
    today_start = datetime(today.year, today.month, today.day)
    today_start_ts = int(today_start.timestamp())
    # End of today (exclusive)
    from datetime import timedelta
    tomorrow = today + timedelta(days=1)
    tomorrow_start = datetime(tomorrow.year, tomorrow.month, tomorrow.day)
    tomorrow_start_ts = int(tomorrow_start.timestamp())

    try:
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        cursor = conn.execute(
            """
            SELECT file_name, timestamp
            FROM transcription_history
            WHERE transcription_text != ''
              AND timestamp >= ? AND timestamp < ?
            """,
            (today_start_ts, tomorrow_start_ts),
        )
        rows = cursor.fetchall()
        conn.close()
    except Exception:
        return 0.0

    rec_dir = get_handy_recordings_dir()
    if rec_dir is None:
        return 0.0

    total_seconds = 0.0
    for file_name, _ts in rows:
        wav_path = rec_dir / file_name
        if wav_path.exists():
            total_seconds += wav_duration(wav_path)

    return total_seconds


def write_daily_progress(db_path: Path | None = None):
    """Write ~/.shadow-companion/daily-progress.json for Perry menubar app.

    Primary metric is TTS playback duration (how long you spent listening/shadowing).
    STT recording duration is included as a secondary field for reference.
    """
    from datetime import date

    tts_seconds = compute_daily_tts_duration()
    stt_seconds = compute_daily_stt_duration(db_path) if db_path else 0.0

    state = load_state()
    target_seconds = state.get("daily_target_s", DEFAULT_DAILY_TARGET_S)
    progress = min(1.0, tts_seconds / target_seconds) if target_seconds > 0 else 0.0

    progress_data = {
        "date": date.today().isoformat(),
        "actual_seconds": round(tts_seconds, 1),
        "stt_seconds": round(stt_seconds, 1),
        "target_seconds": target_seconds,
        "progress": round(progress, 4),
    }

    STATE_DIR.mkdir(parents=True, exist_ok=True)
    DAILY_PROGRESS_FILE.write_text(json.dumps(progress_data, indent=2))


def _verify_progress():
    """Print detailed breakdown of daily progress calculation for verification."""
    from datetime import date, datetime, timedelta

    db_path = find_handy_db()
    if db_path is None:
        print("❌ Could not find Handy's history.db")
        sys.exit(1)

    today = date.today()
    today_start = datetime(today.year, today.month, today.day)
    today_start_ts = int(today_start.timestamp())
    tomorrow = today + timedelta(days=1)
    tomorrow_start = datetime(tomorrow.year, tomorrow.month, tomorrow.day)
    tomorrow_start_ts = int(tomorrow_start.timestamp())

    conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    rows = conn.execute(
        """
        SELECT id, file_name, timestamp, transcription_text
        FROM transcription_history
        WHERE transcription_text != ''
          AND timestamp >= ? AND timestamp < ?
        ORDER BY timestamp ASC
        """,
        (today_start_ts, tomorrow_start_ts),
    ).fetchall()
    conn.close()

    rec_dir = get_handy_recordings_dir()
    state = load_state()
    target_s = state.get("daily_target_s", DEFAULT_DAILY_TARGET_S)

    total = 0.0
    missing = 0

    print(f"Date:          {today.isoformat()}")
    print(f"DB:            {db_path}")
    print(f"Recordings:    {rec_dir}")
    print(f"Time range:    {today_start_ts} — {tomorrow_start_ts}")
    print(f"Target:        {target_s}s ({target_s // 60} min)")
    print(f"Entries today: {len(rows)}")
    print()

    for r in rows:
        wav = rec_dir / r["file_name"] if rec_dir else None
        dur = wav_duration(wav) if wav and wav.exists() else 0.0
        total += dur
        text_preview = r["transcription_text"][:60].replace("\n", " ")
        if wav and wav.exists():
            exists = "✓"
        else:
            exists = "✗ MISSING"
            missing += 1
        print(f"  {r['file_name']}  {dur:>6.1f}s  {exists}  \"{text_preview}...\"")

    # TTS playback duration (primary metric)
    tts_seconds = compute_daily_tts_duration()

    print()
    print(f"STT duration:  {total:.1f}s ({total / 60:.1f} min) — time you spoke into Handy")
    print(f"TTS duration:  {tts_seconds:.1f}s ({tts_seconds / 60:.1f} min) — time you spent listening/shadowing")
    print(f"Target:        {target_s}s ({target_s // 60} min)")
    progress = min(1.0, tts_seconds / target_s) if target_s > 0 else 0.0
    print(f"Progress:      {progress:.4f} ({progress * 100:.1f}%) — based on TTS playback")
    if missing:
        print(f"⚠  {missing} WAV file(s) missing — STT durations not counted")

    # Cross-check with daily-progress.json
    if DAILY_PROGRESS_FILE.exists():
        data = json.loads(DAILY_PROGRESS_FILE.read_text())
        print()
        print(f"daily-progress.json:")
        print(f"  date:            {data.get('date')}")
        print(f"  actual_seconds:  {data.get('actual_seconds')} (TTS playback)")
        print(f"  stt_seconds:     {data.get('stt_seconds', 'N/A')} (STT recording)")
        print(f"  target_seconds:  {data.get('target_seconds')}")
        print(f"  progress:        {data.get('progress')}")
    else:
        print("\n⚠ No daily-progress.json found — run: python shadow.py progress")


# ── Handy DB ──────────────────────────────────────────────────────

def find_handy_db() -> Path | None:
    """Find Handy's history.db on macOS."""
    candidates = [
        Path.home() / "Library" / "Application Support" / "com.pais.handy" / "history.db",
        Path.home() / "Library" / "Application Support" / "com.handy" / "handy" / "history.db",
        Path.home() / "Library" / "Application Support" / "Handy" / "handy" / "history.db",
        Path.home() / "Library" / "Application Support" / "com.handy" / "history.db",
        Path.home() / "Library" / "Application Support" / "Handy" / "history.db",
    ]
    for p in candidates:
        if p.exists():
            return p
    app_support = Path.home() / "Library" / "Application Support"
    for d in app_support.iterdir():
        if "handy" in d.name.lower() and d.is_dir():
            candidate = d / "history.db"
            if candidate.exists():
                return candidate
    return None


def get_latest_entry_id(db_path: Path) -> int:
    try:
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        cursor = conn.execute("SELECT COALESCE(MAX(id), 0) FROM transcription_history")
        row = cursor.fetchone()
        conn.close()
        return row[0] if row else 0
    except Exception:
        return 0


def get_new_entries(db_path: Path, since_id: int) -> list[dict]:
    try:
        conn = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        conn.row_factory = sqlite3.Row
        cursor = conn.execute(
            """
            SELECT id, transcription_text, post_processed_text
            FROM transcription_history
            WHERE id > ? AND transcription_text != ''
            ORDER BY id ASC
            """,
            (since_id,),
        )
        entries = [dict(row) for row in cursor.fetchall()]
        conn.close()
        return entries
    except Exception as e:
        print(f"  ⚠ db read error: {e}")
        return []


# ── TTS ───────────────────────────────────────────────────────────

class ShadowCompanion:
    def __init__(self, voice: str, speed: float, provider: str, db_path: Path):
        self.voice = voice
        self.speed = speed
        self.provider = provider
        self.db_path = db_path
        self.last_id = get_latest_entry_id(db_path)
        self.running = True
        self._state_mtime: float = 0.0

        # TTS worker subprocess — models load in child process for full memory reclamation
        self._model_lock = threading.Lock()
        self._worker_proc = None  # subprocess.Popen or None
        self._last_speak_time = 0.0
        self._worker_ready = threading.Event()  # set when worker prints WORKER_READY

        # Validate provider
        if provider not in ("kokoro", "neutts"):
            print(f"❌ Unknown provider: {provider}. Use 'kokoro' or 'neutts'.")
            sys.exit(1)

        # Save running state
        state = load_state()
        state["running"] = True
        state["voice"] = voice
        state["speed"] = speed
        state["provider"] = provider
        save_state(state)

        print(f"Watching: {db_path}")
        print(f"Ready. TTS worker loads on first utterance. Ctrl+C to quit.")
        if provider == "kokoro":
            print(f"Voice: {voice} | Speed: {speed}x | Provider: {provider}\n")
        else:
            print(f"Provider: {provider} (cloning your voice)\n")



    def _ensure_worker(self):
        """Ensure TTS worker subprocess is running."""
        if self._worker_proc is not None and self._worker_proc.poll() is None:
            return
        with self._model_lock:
            if self._worker_proc is not None and self._worker_proc.poll() is None:
                return
            self._start_worker()

    def _start_worker(self):
        """Start the TTS worker subprocess."""
        # For NeuTTS: ensure reference codes are pre-encoded (torch is only needed
        # for encoding, not inference — encoder runs in a separate subprocess)
        if self.provider == "neutts" and not REF_CODES_FILE.exists():
            print("  🔧 Encoding reference audio (one-time, torch loaded briefly)...")
            venv_python = Path(__file__).parent / ".venv" / "bin" / "python3"
            python = str(venv_python) if venv_python.exists() else sys.executable
            script = Path(__file__).resolve().as_posix()
            enc = subprocess.run(
                [python, "-u", script, "_encode_ref"],
                capture_output=True, text=True, timeout=120,
            )
            if enc.stdout:
                for line in enc.stdout.strip().split("\n"):
                    print(f"    [encoder] {line}")
            if enc.returncode != 0:
                print(f"  ❌ Reference encoding failed: {enc.stderr}")
                return
            print("  ✅ Reference codes saved")

        venv_python = Path(__file__).parent / ".venv" / "bin" / "python3"
        python = str(venv_python) if venv_python.exists() else sys.executable
        script = Path(__file__).resolve().as_posix()

        # Rust worker: prefer if binary exists and provider is neutts
        rust_binary = Path(__file__).parent / "tts-worker-rs" / "target" / "release" / "tts-worker"
        use_rust = (
            self.provider == "neutts"
            and rust_binary.exists()
            and os.environ.get("SHADOW_RUST_TTS", "1") != "0"
        )

        if use_rust:
            worker_argv = [str(rust_binary)]
            print("  🦀 TTS worker (rust) loading models...")
        else:
            worker_argv = [python, "-u", script, "_tts_worker"]

        self._worker_ready.clear()
        self._worker_proc = subprocess.Popen(
            worker_argv,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            bufsize=1,  # line-buffered
            start_new_session=False,  # same process group so SIGTERM kills both
        )
        # Drain worker stdout (loading messages) in background
        threading.Thread(target=self._drain_worker_stdout, daemon=True).start()
        # Wait for worker to finish loading models (up to 120s)
        print("  🔧 TTS worker loading models...")
        if self._worker_ready.wait(timeout=120):
            self._last_speak_time = time.monotonic()
            print("  ✅ TTS worker ready\n")
        else:
            print("  ⚠ TTS worker did not become ready in 120s, will try on next utterance\n")
            self._kill_worker()

    def _drain_worker_stdout(self):
        """Background thread: print worker stdout (loading messages, errors)."""
        proc = self._worker_proc
        if proc is None or proc.stdout is None:
            return
        try:
            for line in proc.stdout:
                line_str = line.decode("utf-8", errors="replace").rstrip()
                if not line_str:
                    continue
                if line_str == "WORKER_READY":
                    self._worker_ready.set()
                else:
                    print(f"  [worker] {line_str}")
        except (ValueError, OSError):
            pass  # pipe closed
        finally:
            # If pipe closed before WORKER_READY, unblock the wait
            self._worker_ready.set()

    def _kill_worker(self):
        """Kill TTS worker subprocess — OS reclaims ALL memory instantly."""
        with self._model_lock:
            if self._worker_proc is None:
                return
            proc = self._worker_proc
            self._worker_proc = None
            self._worker_ready.clear()
            try:
                proc.terminate()
                try:
                    proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=2)
            except (ProcessLookupError, OSError):
                pass
            print("  💤 TTS worker killed (idle timeout) — memory freed\n")

    def _idle_unloader(self):
        """Background thread: kill TTS worker after IDLE_TIMEOUT_S,
        and periodically refresh daily progress file."""
        cycle = 0
        while self.running:
            time.sleep(60)
            cycle += 1
            with self._model_lock:
                proc = self._worker_proc
                last = self._last_speak_time
            if proc is not None and last > 0:
                idle_seconds = time.monotonic() - last
                if idle_seconds > IDLE_TIMEOUT_S:
                    self._kill_worker()
            # Refresh progress every 5 minutes (the TTS play log may have
            # been updated by the worker since last DB-triggered refresh)
            if cycle % 5 == 0:
                try:
                    write_daily_progress(self.db_path)
                except Exception:
                    pass

    def speak_streaming(self, text: str):
        """Send text to TTS worker subprocess for playback."""
        text = text.strip()
        if not text:
            return
        if len(text) > 500:
            text = text[:500] + "..."
            print(f"  ⚠ truncated to 500 chars")
        print(f"  ▶ {text[:80]}{'...' if len(text) > 80 else ''}")

        self._ensure_worker()
        with self._model_lock:
            proc = self._worker_proc
        if proc is None or proc.poll() is not None or proc.stdin is None:
            print("  ✗ TTS worker not available\n")
            return

        try:
            # Send text line to worker stdin (JSON-encoded for safety)
            payload = json.dumps({"text": text}) + "\n"
            proc.stdin.write(payload.encode("utf-8"))
            proc.stdin.flush()
        except (BrokenPipeError, OSError):
            print("  ✗ TTS worker crashed, will restart on next utterance\n")
            self._kill_worker()
            return

        self._last_speak_time = time.monotonic()



    def _watch_db_kqueue(self):
        """Watch DB for changes using kqueue (macOS native, zero-polling-delay)."""
        import select
        import os

        while self.running:
            # Hot-reload config
            self._check_config_reload()

            # Check for new entries first
            entries = get_new_entries(self.db_path, self.last_id)
            for entry in entries:
                if not self.running:
                    return
                text = entry.get("post_processed_text") or entry.get("transcription_text", "")
                if text.strip():
                    self.speak_streaming(text.strip())
                self.last_id = entry["id"]

            # Update daily progress after processing new entries.
            # Small delay so the worker has time to play audio and log
            # the duration to tts-play-log.json before we read it.
            if entries:
                time.sleep(0.5)
                write_daily_progress(self.db_path)

            # Wait for DB change via kqueue
            try:
                kq = select.kqueue()
                fd = os.open(str(self.db_path), os.O_RDONLY)
                kev = select.kevent(
                    fd,
                    filter=select.KQ_FILTER_VNODE,
                    flags=select.KQ_EV_ADD | select.KQ_EV_ENABLE | select.KQ_EV_CLEAR,
                    fflags=select.KQ_NOTE_WRITE | select.KQ_NOTE_EXTEND,
                )
                # Block until DB changes or timeout (for config hot-reload)
                events = kq.control([kev], 1, 2.0)
                kq.close()
                os.close(fd)
                if events:
                    # Small sleep to let Handy finish writing
                    time.sleep(0.05)
            except (OSError, FileNotFoundError):
                # DB file might have been recreated — fall back to polling
                time.sleep(POLL_S)

    def _watch_db_poll(self):
        """Watch DB for changes using polling (fallback)."""
        while self.running:
            self._check_config_reload()

            entries = get_new_entries(self.db_path, self.last_id)
            for entry in entries:
                if not self.running:
                    return
                text = entry.get("post_processed_text") or entry.get("transcription_text", "")
                if text.strip():
                    self.speak_streaming(text.strip())
                self.last_id = entry["id"]

            # Update daily progress after processing new entries.
            # Small delay so the worker has time to play audio and log
            # the duration to tts-play-log.json before we read it.
            if entries:
                time.sleep(0.5)
                write_daily_progress(self.db_path)

            time.sleep(POLL_S)

    def _check_config_reload(self):
        """Hot-reload voice/speed from state file if changed (Kokoro only)."""
        try:
            st = os.stat(STATE_FILE)
            mtime = st.st_mtime
        except OSError:
            return
        if mtime == self._state_mtime:
            return
        self._state_mtime = mtime
        state = load_state()

        # Provider changes require restart — skip hot-reload
        if state.get("provider") != self.provider:
            if state.get("provider") is not None:
                print(f"  ⚠ Provider changed: {self.provider} → {state['provider']}. Restart required.")
            return

        if self.provider != "kokoro":
            return

        voice_changed = state.get("voice") != self.voice
        speed_changed = state.get("speed") != self.speed

        if voice_changed or speed_changed:
            if voice_changed:
                print(f"  🔄 Voice changed: {self.voice} → {state['voice']}")
                self.voice = state["voice"]
            if speed_changed:
                print(f"  🔄 Speed changed: {self.speed} → {state.get('speed', 1.0)}")
                self.speed = state.get("speed", 1.0)

            if self._worker_proc is not None and self._worker_proc.poll() is None:
                self._kill_worker()
                print("  TTS worker will restart with new config on next utterance")
            else:
                print(f"  ✓ Config updated (worker not running)\n")

    def run(self):
        """Start watching DB — uses kqueue if available, else polling."""
        unloader = threading.Thread(target=self._idle_unloader, daemon=True)
        unloader.start()
        try:
            import select
            if hasattr(select, 'kqueue') and hasattr(select, 'KQ_FILTER_VNODE'):
                print("  Using kqueue for DB watching (instant detection)")
                self._watch_db_kqueue()
            else:
                self._watch_db_poll()
        except ImportError:
            self._watch_db_poll()

    def stop(self):
        self.running = False
        self._kill_worker()
        state = load_state()
        state["running"] = False
        save_state(state)


# ── server management ─────────────────────────────────────────────

def is_server_running() -> bool:
    if not PID_FILE.exists():
        return False
    try:
        pid = int(PID_FILE.read_text().strip())
        os.kill(pid, 0)  # Check if process exists
        return True
    except (ProcessLookupError, ValueError, PermissionError):
        PID_FILE.unlink(missing_ok=True)
        return False


def start_server(voice: str, speed: float, provider: str):
    if is_server_running():
        print("Shadow Companion is already running.")
        print("Use 'python shadow.py stop' first, or 'python shadow.py restart'.")
        sys.exit(1)

    db_path = find_handy_db()
    if db_path is None:
        print("❌ Could not find Handy's history.db")
        sys.exit(1)

    # Save config
    state = load_state()
    state["voice"] = voice
    state["speed"] = speed
    state["provider"] = provider
    save_state(state)

    # Start as background process
    venv_python = Path(__file__).parent / ".venv" / "bin" / "python3"
    python = str(venv_python) if venv_python.exists() else sys.executable

    log_file = STATE_DIR / "server.log"
    STATE_DIR.mkdir(parents=True, exist_ok=True)

    proc = subprocess.Popen(
        [python, "-u", Path(__file__).resolve().as_posix(), "_run_server"],
        stdout=open(log_file, "a"),
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )

    # With start_new_session=True, PGID == PID
    PID_FILE.write_text(str(proc.pid))
    print(f"✅ Shadow Companion started (PID {proc.pid})")
    if provider == "neutts":
        print(f"   Provider: {provider} (voice cloning) | Log: {log_file}")
    else:
        print(f"   Voice: {voice} | Speed: {speed}x | Log: {log_file}")


def stop_server():
    if not is_server_running():
        print("Shadow Companion is not running.")
        # Still clean up stale state
        state = load_state()
        state["running"] = False
        save_state(state)
        PID_FILE.unlink(missing_ok=True)
        sys.exit(0)

    pid = int(PID_FILE.read_text().strip())
    try:
        # Kill the whole process group (child + any subprocesses)
        try:
            os.killpg(pid, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            os.kill(pid, signal.SIGTERM)
        # Wait for process to die
        for _ in range(15):
            try:
                os.kill(pid, 0)
                time.sleep(0.3)
            except ProcessLookupError:
                break
        else:
            # Force kill if still alive
            try:
                os.killpg(pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                os.kill(pid, signal.SIGKILL)
        print("✅ Shadow Companion stopped.")
    except ProcessLookupError:
        print("Process already gone.")
    finally:
        PID_FILE.unlink(missing_ok=True)
        state = load_state()
        state["running"] = False
        save_state(state)


def server_status():
    if is_server_running():
        pid = int(PID_FILE.read_text().strip())
        state = load_state()
        provider = state.get('provider', 'kokoro')
        if provider == 'neutts':
            print(f"🟢 Shadow Companion is running (PID {pid})")
            print(f"   Provider: {provider} (voice cloning)")
        else:
            print(f"🟢 Shadow Companion is running (PID {pid})")
            print(f"   Voice: {state.get('voice', 'am_michael')} | Speed: {state.get('speed', 1.0)}x")
    else:
        print("🔴 Shadow Companion is not running.")


def _run_server():
    """Internal: actual server loop, called by start_server as subprocess."""
    state = load_state()
    db_path = find_handy_db()
    if db_path is None:
        print("❌ Could not find Handy's history.db")
        sys.exit(1)

    provider = state.get("provider", "kokoro")
    companion = ShadowCompanion(
        voice=state.get("voice", "am_michael"),
        speed=state.get("speed", 1.0),
        provider=provider,
        db_path=db_path,
    )

    # Write initial daily progress
    write_daily_progress(db_path)

    def handle_sigterm(sig, frame):
        companion.stop()
        PID_FILE.unlink(missing_ok=True)
        # Kill own process group to ensure all children die
        try:
            os.killpg(os.getpid(), signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            sys.exit(0)

    signal.signal(signal.SIGTERM, handle_sigterm)
    signal.signal(signal.SIGINT, handle_sigterm)
    companion.run()


# ── NeuTTS voice setup ─────────────────────────────────────────────

def _setup_voice():
    """Interactive onboarding: record reference audio for voice cloning."""
    import sounddevice as sd
    import numpy as np

    STATE_DIR.mkdir(parents=True, exist_ok=True)

    print("🎙️  NeuTTS Voice Setup")
    print("=" * 40)
    print()
    print("You'll record a short clip of your natural speaking voice.")
    print("This will be used to clone your voice for TTS playback.")
    print()
    print("Tips for best results:")
    print("  • Speak naturally, at your normal pace")
    print("  • Use the language you'll be shadowing (English)")
    print("  • 3-15 seconds, continuous speech, no long pauses")
    print("  • Quiet environment")
    print()

    # Check if existing reference exists
    if REF_VOICE_WAV.exists():
        print(f"Existing reference found: {REF_VOICE_WAV}")
        overwrite = input("Overwrite? [y/N] ").strip().lower()
        if overwrite != 'y':
            print("Keeping existing reference.")
            return

    # Get recording duration
    print()
    duration = input("Recording duration in seconds (5-15, default 10): ").strip()
    try:
        duration = max(3, min(15, int(duration)))
    except ValueError:
        duration = 10

    # Countdown
    print(f"\nRecording {duration}s of your voice...")
    for i in range(3, 0, -1):
        print(f"  {i}...")
        time.sleep(1)
    print("  🎙️  GO!")

    # Record
    sample_rate = 16000  # NeuTTS expects 16kHz for encode_reference
    recording = sd.rec(int(duration * sample_rate), samplerate=sample_rate, channels=1, dtype='float32')
    sd.wait()
    print("  ✓ Recording complete.")

    # Playback for verification
    print("\nPlaying back your recording...")
    sd.play(recording.flatten(), sample_rate)
    sd.wait()

    accept = input("\nUse this recording? [Y/n] ").strip().lower()
    if accept == 'n':
        print("Cancelled. Run setup-voice again to retry.")
        return

    # Save audio
    import soundfile as sf
    sf.write(str(REF_VOICE_WAV), recording.flatten(), sample_rate)
    print(f"\n✅ Audio saved to {REF_VOICE_WAV}")

    # Get reference text
    print()
    print("Now type exactly what you said in the recording.")
    print("(This is needed by the model for voice cloning accuracy.)")
    if REF_VOICE_TXT.exists():
        existing = REF_VOICE_TXT.read_text().strip()
        print(f"Current: \"{existing}\"")
    ref_text = input("Reference text: ").strip()
    if not ref_text:
        print("❌ Reference text is required. Run setup-voice again.")
        return

    REF_VOICE_TXT.write_text(ref_text + "\n")
    print(f"✅ Reference text saved to {REF_VOICE_TXT}")

    print()
    print("🎉 Voice setup complete!")
    print(f"   Audio: {REF_VOICE_WAV}")
    print(f"   Text:  {REF_VOICE_TXT}")
    print()
    print("To use NeuTTS, run:")
    print("   python shadow.py set-provider neutts")
    print("   python shadow.py restart")


# ── Reference encoder subprocess ────────────────────────────────────

REF_CODES_FILE = STATE_DIR / "ref_codes.npy"


def _encode_ref_subprocess():
    """Short-lived subprocess: load torch + PyTorch codec, encode reference
    audio, save ref_codes.npy, exit. OS reclaims ALL torch memory on exit."""
    ref_audio = REF_VOICE_WAV
    ref_text_path = REF_VOICE_TXT
    if not ref_audio.exists():
        print(f"No reference audio at {ref_audio}")
        sys.exit(1)
    ref_text = None
    if ref_text_path and ref_text_path.exists():
        ref_text = ref_text_path.read_text().strip()
    if not ref_text:
        print(f"No reference text at {ref_text_path}")
        sys.exit(1)

    # Load torch + codec encoder (this is the expensive part)
    from neutts import NeuTTS
    neutts = NeuTTS(
        backbone_repo="neuphonic/neutts-air-q8-gguf",
        backbone_device="cpu",
        codec_repo="neuphonic/neucodec",
        codec_device="cpu",
    )
    print("Encoding reference audio...")
    ref_codes = neutts.encode_reference(str(ref_audio))

    # Save to disk — torch not needed after this
    import numpy as np
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    np.save(str(REF_CODES_FILE), ref_codes.numpy() if hasattr(ref_codes, 'numpy') else np.array(ref_codes))
    print(f"Reference codes saved to {REF_CODES_FILE}")

    # Also save ref_text for the worker to pick up
    (STATE_DIR / "ref_text.txt").write_text(ref_text)
    print("Encoder subprocess done — torch memory will be reclaimed on exit")


# ── TTS worker subprocess ──────────────────────────────────────────

def _tts_worker_main():
    """Internal: TTS worker subprocess. Reads text lines from stdin,
    generates audio, plays it. All heavy model memory is isolated here
    so killing this process frees ALL RAM instantly."""
    # Handle SIGTERM gracefully — stop audio and exit
    import numpy as np
    import sounddevice as sd
    _worker_shutdown = False

    def _worker_sigterm(sig, frame):
        nonlocal _worker_shutdown
        _worker_shutdown = True
        try:
            sd.stop()
        except Exception:
            pass
        sys.exit(0)

    signal.signal(signal.SIGTERM, _worker_sigterm)

    state = load_state()
    provider = state.get("provider", "kokoro")
    voice = state.get("voice", "am_michael")
    speed = state.get("speed", 1.0)

    # Load models (this is where the RAM goes)
    if provider == "kokoro":
        from pykokoro import build_pipeline, PipelineConfig
        from pykokoro.generation_config import GenerationConfig
        from pykokoro.stages.doc_parsers.ssmd import SsmdDocumentParser
        from pykokoro.stages.g2p.kokorog2p import KokoroG2PAdapter
        from pykokoro.stages.phoneme_processing.onnx import OnnxPhonemeProcessorAdapter
        from pykokoro.stages.audio_generation.onnx import OnnxAudioGenerationAdapter
        from pykokoro.stages.audio_postprocessing.onnx import OnnxAudioPostprocessingAdapter
        from pykokoro.runtime.tracing import Trace
        from pykokoro.constants import SAMPLE_RATE
        from pykokoro.types import Segment
        from dataclasses import replace as dc_replace

        print(f"Loading Kokoro model (voice={voice})...")
        pipe = build_pipeline(
            config={"voice": voice, "provider": provider, "generation": {"speed": speed}},
            eager=True,
        )
        print("Pre-warming TTS engine...")
        pipe.run("Ready.")
        print("Kokoro ready")

    elif provider == "neutts":
        import soundfile as sf

        # ── Torch-free NeuTTS path ──────────────────────────────────
        # Strategy: load ref_codes from disk (encoded in a separate short-lived
        # subprocess that loaded torch, encoded, saved, exited → OS reclaimed torch).
        # The worker only needs llama_cpp (GGUF backbone) + onnxruntime (codec decoder)
        # + phonemizer — no torch/transformers at all.

        # Check ref_codes exist
        if not REF_CODES_FILE.exists():
            print(f"No reference codes at {REF_CODES_FILE}")
            print(f"Run: python shadow.py _encode_ref")
            sys.exit(1)

        ref_text_path = STATE_DIR / "ref_text.txt"
        if not ref_text_path.exists():
            print(f"No reference text at {ref_text_path}")
            sys.exit(1)
        ref_text = ref_text_path.read_text().strip()

        # Load pre-computed reference codes (numpy, no torch needed)
        ref_codes = np.load(str(REF_CODES_FILE))
        print(f"Loaded reference codes: shape={ref_codes.shape}")

        # ── Stub out heavy deps BEFORE any import that touches them ───
        # neutts.py and neucodec/model.py do `import torch` at module level.
        # We replace torch/transformers/torchaudio with stubs so those imports
        # succeed without loading the real 237 MB libtorch.
        import types

        # Stub base class (neucodec.model.py: class NeuCodec(nn.Module))
        class _StubModule:
            def __init__(self, *a, **kw): pass
            def eval(self): return self
            def to(self, *a, **kw): return self
            def __call__(self, *a, **kw): return None

        def _make_torch_stub():
            """Create a torch module stub that supports the subset used by
            neucodec.model.py, neutts.neutts.py, and huggingface_hub at import time."""
            t = types.ModuleType('torch')
            nn_stub = types.ModuleType('torch.nn')
            nn_stub.Module = _StubModule
            nn_stub.functional = types.ModuleType('torch.nn.functional')
            nn_stub.functional.pad = lambda *a, **kw: None
            # torch.nn.utils (neucodec/module.py: from torch.nn.utils import weight_norm)
            nn_utils = types.ModuleType('torch.nn.utils')
            nn_utils.weight_norm = lambda x: x
            nn_stub.utils = nn_utils
            # Common nn layers used at class-def time in neucodec
            nn_stub.Linear = _StubModule
            nn_stub.Conv1d = _StubModule
            nn_stub.Conv2d = _StubModule
            nn_stub.ConvTranspose1d = _StubModule
            nn_stub.Embedding = _StubModule
            nn_stub.Sequential = _StubModule
            nn_stub.ModuleList = _StubModule
            nn_stub.Parameter = lambda *a, **kw: None
            t.nn = nn_stub
            t.Tensor = _StubModule
            t.device = lambda x: x
            t.no_grad = lambda: type('ctx', (), {'__enter__': lambda s: None, '__exit__': lambda s, *a: None})()
            t.from_numpy = lambda x: x
            t.tensor = lambda *a, **kw: None
            t.load = lambda *a, **kw: {}
            t.vstack = lambda *a, **kw: None
            t.cat = lambda *a, **kw: None
            t.nn.functional = nn_stub.functional
            # Dtype constants (needed by huggingface_hub.serialization._torch)
            t.int64 = int
            t.int32 = int
            t.float16 = float
            t.float32 = float
            t.float64 = float
            t.bfloat16 = float
            t.bool = bool
            t.uint8 = int
            t.int8 = int
            t.int16 = int
            t.complex64 = complex
            t.complex128 = complex
            return t

        _torch_stub = _make_torch_stub()
        sys.modules['torch'] = _torch_stub
        sys.modules['torch.nn'] = _torch_stub.nn
        sys.modules['torch.nn.functional'] = _torch_stub.nn.functional
        sys.modules['torch.nn.utils'] = _torch_stub.nn.utils

        # torchaudio stub
        _torchaudio_stub = types.ModuleType('torchaudio')
        _torchaudio_stub.transforms = types.ModuleType('torchaudio.transforms')
        sys.modules['torchaudio'] = _torchaudio_stub
        sys.modules['torchaudio.transforms'] = _torchaudio_stub.transforms

        # transformers stub
        _transformers_stub = types.ModuleType('transformers')
        _transformers_stub.AutoTokenizer = type('AutoTokenizer', (), {'from_pretrained': staticmethod(lambda *a, **kw: None)})
        _transformers_stub.AutoModelForCausalLM = type('AutoModelForCausalLM', (), {'from_pretrained': staticmethod(lambda *a, **kw: None)})
        _transformers_stub.AutoFeatureExtractor = type('AutoFeatureExtractor', (), {'from_pretrained': staticmethod(lambda *a, **kw: None)})
        _transformers_stub.HubertModel = _StubModule
        _transformers_stub.Wav2Vec2BertModel = _StubModule
        sys.modules['transformers'] = _transformers_stub
        for sub in ['models', 'models.auto']:
            sys.modules[f'transformers.{sub}'] = types.ModuleType(f'transformers.{sub}')

        # Pre-stub huggingface_hub to prevent it from trying to use torch
        # during lazy import of hub_mixin. Must happen BEFORE neucodec import
        # because neucodec/model.py does `from huggingface_hub import PyTorchModelHubMixin, ModelHubMixin`.
        import huggingface_hub as _hf
        if not hasattr(_hf, 'PyTorchModelHubMixin'):
            _hf.PyTorchModelHubMixin = _StubModule
        if not hasattr(_hf, 'ModelHubMixin'):
            _hf.ModelHubMixin = _StubModule
        # Ensure the lazy-import submodules are also pre-loaded
        if 'huggingface_hub.hub_mixin' not in sys.modules:
            _hm = types.ModuleType('huggingface_hub.hub_mixin')
            _hm.PyTorchModelHubMixin = _StubModule
            _hm.ModelHubMixin = _StubModule
            sys.modules['huggingface_hub.hub_mixin'] = _hm

        # ── Load NeuCodecOnnxDecoder WITHOUT importing the neucodec package ───
        # The neucodec package drags in torch at module level via its __init__.py
        # which imports NeuCodec (torch-dependent). Instead, we create a minimal
        # class that does exactly what NeuCodecOnnxDecoder does: load an ONNX
        # session from a HuggingFace cache path.
        import onnxruntime
        from huggingface_hub import hf_hub_download

        class _NeuCodecOnnxDecoder:
            """Torch-free ONNX codec decoder. Reimplements neucodec.NeuCodecOnnxDecoder
            without the torch-dependent import chain."""
            def __init__(self, onnx_path):
                so = onnxruntime.SessionOptions()
                so.graph_optimization_level = onnxruntime.GraphOptimizationLevel.ORT_ENABLE_ALL
                self.session = onnxruntime.InferenceSession(onnx_path, sess_options=so)
                self.sample_rate = 24_000

            def decode_code(self, codes: np.ndarray) -> np.ndarray:
                if not isinstance(codes, np.ndarray):
                    raise ValueError("Codes should be an np.array.")
                if not len(codes.shape) == 3 or codes.shape[1] != 1:
                    raise ValueError("Codes should be of shape [B, 1, F].")
                return self.session.run(None, {"codes": codes})[0].astype(np.float32)

            @classmethod
            def from_pretrained(cls, model_id: str, **kwargs):
                onnx_path = hf_hub_download(repo_id=model_id, filename="model.onnx", **kwargs)
                # Download meta.yaml for download tracking (same as upstream)
                try:
                    hf_hub_download(repo_id=model_id, filename="meta.yaml", **kwargs)
                except Exception:
                    pass
                return cls(onnx_path)

        # Stub the neucodec package so neutts can import it if needed
        _neucodec_stub = types.ModuleType('neucodec')
        _neucodec_stub.NeuCodecOnnxDecoder = _NeuCodecOnnxDecoder
        _neucodec_stub.NeuCodec = _StubModule
        _neucodec_stub.DistillNeuCodec = _StubModule
        sys.modules['neucodec'] = _neucodec_stub

        # Now safe to import neutts (it may import neucodec, but gets our stub)
        from neutts import NeuTTS

        # Disable mlock — let OS swap out model when memory-pressured
        import llama_cpp
        _orig_llama_init = llama_cpp.Llama.__init__
        def _patched_init(self_llama, *args, **kwargs):
            kwargs['mlock'] = False
            return _orig_llama_init(self_llama, *args, **kwargs)
        llama_cpp.Llama.__init__ = _patched_init

        # Load NeuTTS with ONNX codec from the start (no PyTorch codec)
        print("Loading NeuTTS Air Q8 + ONNX codec (torch-free)...")
        neutts = NeuTTS(
            backbone_repo="neuphonic/neutts-air-q8-gguf",
            backbone_device="cpu",
            codec_repo="neuphonic/neucodec-onnx-decoder",
            codec_device="cpu",
        )
        llama_cpp.Llama.__init__ = _orig_llama_init

        print("Pre-warming TTS engine...")
        neutts.infer("Ready.", ref_codes, ref_text)
        print("NeuTTS ready (torch-free)")

    print("WORKER_READY")  # Signal to parent that models are loaded
    sys.stdout.flush()

    # Main loop: read text from stdin, speak it
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            payload = json.loads(line)
            text = payload.get("text", "").strip()
        except (json.JSONDecodeError, KeyError):
            text = line  # fallback: raw text

        if not text:
            continue
        if len(text) > 500:
            text = text[:500] + "..."

        try:
            if provider == "kokoro":
                _speak_kokoro_in_worker(pipe, np, sd, text, SsmdDocumentParser, KokoroG2PAdapter,
                    OnnxPhonemeProcessorAdapter, OnnxAudioGenerationAdapter,
                    OnnxAudioPostprocessingAdapter, Trace, SAMPLE_RATE, Segment)
            elif provider == "neutts":
                _speak_neutts_in_worker(neutts, np, sd, ref_codes, ref_text, text)
        except Exception as e:
            print(f"TTS error: {e}")

        sys.stdout.flush()


def _speak_kokoro_in_worker(pipe, np, sd, text, SsmdDocumentParser, KokoroG2PAdapter,
                            OnnxPhonemeProcessorAdapter, OnnxAudioGenerationAdapter,
                            OnnxAudioPostprocessingAdapter, Trace, SAMPLE_RATE, Segment):
    """Kokoro TTS playback (runs in worker subprocess)."""
    config = pipe.config
    trace = Trace()

    doc = SsmdDocumentParser().parse(text, config, trace)
    segments = doc.segments
    if not segments and doc.clean_text:
        segments = [Segment(
            id="p0_s0_c0_seg0", text=doc.clean_text,
            char_start=0, char_end=len(doc.clean_text),
            paragraph_idx=0, sentence_idx=0, clause_idx=0,
        )]
    phoneme_segments = KokoroG2PAdapter().phonemize(segments, doc, config, trace)

    # Fast path: single segment
    if len(phoneme_segments) <= 1:
        res = pipe.run(text)
        if res.audio is None or len(res.audio) == 0:
            return
        audio = res.audio.astype(np.float32) if hasattr(res.audio, 'astype') else np.array(res.audio, dtype=np.float32)
        duration = len(audio) / res.sample_rate
        sd.play(audio, res.sample_rate)
        sd.wait()
        log_tts_play(duration)
        print(f"{duration:.1f}s played")
        return

    # Streaming path: multiple segments
    kokoro, _ = pipe._ensure_kokoro(config)
    pp = OnnxPhonemeProcessorAdapter(kokoro)
    phoneme_segments = pp.process(phoneme_segments, config, trace)
    ag = OnnxAudioGenerationAdapter(kokoro)
    ap = OnnxAudioPostprocessingAdapter(kokoro)

    total_duration = 0.0
    for seg in phoneme_segments:
        seg_result = ag.generate([seg], config, trace)
        audio = ap.postprocess(seg_result, config, trace)
        if audio is not None and len(audio) > 0:
            audio_f32 = audio.astype(np.float32)
            dur = len(audio_f32) / SAMPLE_RATE
            total_duration += dur
            sd.wait()
            sd.play(audio_f32, SAMPLE_RATE)

    sd.wait()
    if total_duration > 0:
        log_tts_play(total_duration)
        print(f"{total_duration:.1f}s played")


def _speak_neutts_in_worker(neutts, np, sd, ref_codes, ref_text, text):
    """NeuTTS streaming playback (runs in worker subprocess)."""
    import queue as _queue

    sample_rate = neutts.sample_rate  # 24000
    chunk_queue = _queue.Queue()

    buf = np.zeros(0, dtype=np.float32)
    buf_offset = 0
    got_sentinel = False

    def _callback(outdata, frames, time_info, status):
        nonlocal buf, buf_offset, got_sentinel
        if got_sentinel:
            outdata[:] = 0
            raise sd.CallbackStop
        written = 0
        while written < frames:
            avail = len(buf) - buf_offset
            if avail > 0:
                n = min(avail, frames - written)
                outdata[written:written + n, 0] = buf[buf_offset:buf_offset + n]
                buf_offset += n
                written += n
            else:
                try:
                    chunk = chunk_queue.get(timeout=2.0)
                except _queue.Empty:
                    outdata[written:, 0] = 0.0
                    return
                if chunk is None:  # sentinel
                    got_sentinel = True
                    outdata[written:, 0] = 0.0
                    raise sd.CallbackStop
                buf = chunk
                buf_offset = 0

    stream = sd.OutputStream(
        samplerate=sample_rate, channels=1, dtype='float32',
        callback=_callback, blocksize=1024,
    )
    stream.start()

    total_samples = 0
    for chunk in neutts.infer_stream(text, ref_codes, ref_text):
        if chunk is not None and len(chunk) > 0:
            audio = chunk.astype(np.float32)
            chunk_queue.put(audio)
            total_samples += len(audio)

    # Tail padding + sentinel
    chunk_queue.put(np.zeros(int(0.15 * sample_rate), dtype=np.float32))
    chunk_queue.put(None)

    total_duration = total_samples / sample_rate
    timeout = max(total_duration + 5, 10)
    deadline = time.monotonic() + timeout
    while stream.active and time.monotonic() < deadline:
        time.sleep(0.05)

    stream.stop()
    stream.close()

    if total_duration > 0:
        log_tts_play(total_duration)
        print(f"{total_duration:.1f}s played")


# ── CLI ───────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="Shadow Companion — TTS echo for language shadowing with Handy"
    )
    sub = parser.add_subparsers(dest="command")

    # Direct run (foreground)
    parser.add_argument("--voice", default="am_michael", choices=VOICE_LIST)
    parser.add_argument("--speed", type=float, default=1.0)
    parser.add_argument("--provider", default="kokoro", choices=["kokoro", "neutts"])
    parser.add_argument("--db", default=None)

    # Server commands
    sub.add_parser("serve", help="Start as background server")
    sub.add_parser("stop", help="Stop running server")
    sub.add_parser("status", help="Check server status")
    sub.add_parser("restart", help="Restart server")

    sub.add_parser("progress", help="Compute and print daily STT progress")
    sub.add_parser("verify", help="Show detailed breakdown of daily progress calculation")

    set_daily_target = sub.add_parser("set-daily-target", help="Set daily target in minutes")
    set_daily_target.add_argument("minutes", type=int)

    set_voice = sub.add_parser("set-voice", help="Change voice (hot-reloads if server running)")
    set_voice.add_argument("voice", choices=VOICE_LIST)

    set_speed = sub.add_parser("set-speed", help="Change speech speed (Kokoro only)")
    set_speed.add_argument("speed", type=float)

    set_provider = sub.add_parser("set-provider", help="Change TTS engine (requires restart)")
    set_provider.add_argument("provider", choices=["kokoro", "neutts"])

    sub.add_parser("setup-voice", help="Record reference audio for NeuTTS voice cloning")

    # Internal
    sub.add_parser("_run_server", help=argparse.SUPPRESS)
    sub.add_parser("_tts_worker", help=argparse.SUPPRESS)
    sub.add_parser("_encode_ref", help=argparse.SUPPRESS)

    args = parser.parse_args()

    # Internal server command
    if args.command == "_run_server":
        _run_server()
        return

    # Internal TTS worker subprocess
    if args.command == "_tts_worker":
        _tts_worker_main()
        return

    # Internal: encode reference in short-lived subprocess
    if args.command == "_encode_ref":
        _encode_ref_subprocess()
        return

    # Server management commands
    if args.command == "serve":
        state = load_state()
        start_server(
            voice=state.get("voice", args.voice),
            speed=state.get("speed", args.speed),
            provider=state.get("provider", args.provider),
        )
        return

    if args.command == "stop":
        stop_server()
        return

    if args.command == "status":
        server_status()
        return

    if args.command == "restart":
        if is_server_running():
            stop_server()
            time.sleep(0.5)
        state = load_state()
        start_server(
            voice=state.get("voice", args.voice),
            speed=state.get("speed", args.speed),
            provider=state.get("provider", args.provider),
        )
        return

    if args.command == "set-voice":
        state = load_state()
        state["voice"] = args.voice
        save_state(state)
        print(f"✅ Voice set to {args.voice}")
        if is_server_running():
            print("   Server will hot-reload on next poll cycle.")
        return

    if args.command == "set-speed":
        state = load_state()
        state["speed"] = args.speed
        save_state(state)
        print(f"✅ Speed set to {args.speed}x")
        if state.get("provider") == "neutts":
            print("   ⚠ Speed control is not available with NeuTTS provider.")
        return

    if args.command == "set-provider":
        state = load_state()
        old_provider = state.get("provider", "kokoro")
        state["provider"] = args.provider
        save_state(state)
        print(f"✅ Provider set to {args.provider}")
        if args.provider == "neutts" and not REF_VOICE_WAV.exists():
            print(f"   ⚠ No reference audio found. Run: python shadow.py setup-voice")
        if is_server_running():
            if old_provider != args.provider:
                print("   ⚠ Provider change requires restart. Run: python shadow.py restart")
            else:
                print("   Server will hot-reload on next poll cycle.")
        return

    if args.command == "setup-voice":
        _setup_voice()
        return

    if args.command == "progress":
        db_path = find_handy_db()
        if db_path is None:
            print("❌ Could not find Handy's history.db")
            sys.exit(1)
        write_daily_progress(db_path)
        if DAILY_PROGRESS_FILE.exists():
            data = json.loads(DAILY_PROGRESS_FILE.read_text())
            actual_min = data["actual_seconds"] / 60
            target_min = data["target_seconds"] / 60
            pct = data["progress"] * 100
            print(f"📊 {actual_min:.1f}/{target_min:.0f} min ({pct:.0f}%) — {data['date']}")
        else:
            print("❌ Could not compute progress")
        return

    if args.command == "verify":
        _verify_progress()
        return

    if args.command == "set-daily-target":
        state = load_state()
        state["daily_target_s"] = args.minutes * 60
        save_state(state)
        print(f"✅ Daily target set to {args.minutes} minutes")
        # Recompute progress with new target
        db_path = find_handy_db()
        if db_path:
            write_daily_progress(db_path)
        return

    # Default: direct foreground run
    db_path = Path(args.db) if args.db else find_handy_db()
    if db_path is None:
        print("❌ Could not find Handy's history.db")
        print("   Make sure Handy is installed and has been used at least once.")
        print("   Or specify the path manually: python shadow.py --db /path/to/history.db")
        sys.exit(1)

    companion = ShadowCompanion(
        voice=args.voice,
        speed=args.speed,
        provider=args.provider,
        db_path=db_path,
    )

    def handle_sigint(sig, frame):
        print("\nStopping...")
        companion.stop()
        sys.exit(0)

    signal.signal(signal.SIGINT, handle_sigint)
    companion.run()


if __name__ == "__main__":
    main()
