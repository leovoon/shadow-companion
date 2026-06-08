#!/usr/bin/env python3
"""
Shadow Companion — watches Handy's transcription history database,
speaks new entries back with Kokoro TTS so you can shadow native intonation.

Usage:
    python shadow.py [--voice VOICE] [--speed SPEED] [--provider PROVIDER]

Server mode (for Raycast/CLI control):
    python shadow.py serve                    # start as background server
    python shadow.py stop                     # stop running server
    python shadow.py status                   # check if server is running
    python shadow.py set-voice <voice>        # change voice (hot-reloads)
    python shadow.py set-speed <speed>        # change speed
"""

import argparse
import json
import os
import signal
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

# ── voices ────────────────────────────────────────────────────────
VOICE_LIST = [
    "af_heart", "af_nicole", "af_sarah", "af_bella", "af_river",
    "af_sky", "af_nova", "af_alloy", "af_aoede", "af_kore",
    "am_michael", "am_adam", "am_eric", "am_liam", "am_onyx",
    "am_puck", "am_echo", "am_fenrir",
]

POLL_S = 0.5

# Server state file
STATE_DIR = Path.home() / ".shadow-companion"
STATE_FILE = STATE_DIR / "state.json"
PID_FILE = STATE_DIR / "server.pid"


# ── state management ──────────────────────────────────────────────

def load_state() -> dict:
    if STATE_FILE.exists():
        try:
            return json.loads(STATE_FILE.read_text())
        except Exception:
            pass
    return {"voice": "am_michael", "speed": 1.0, "provider": "cpu", "running": False}


def save_state(state: dict):
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    STATE_FILE.write_text(json.dumps(state, indent=2))


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
        import numpy as np
        import sounddevice as sd
        from pykokoro import KokoroPipeline, PipelineConfig
        from pykokoro.generation_config import GenerationConfig
        from dataclasses import replace as dc_replace

        self._np = np
        self._sd = sd
        self._KokoroPipeline = KokoroPipeline
        self._PipelineConfig = PipelineConfig
        self._GenerationConfig = GenerationConfig
        self._dc_replace = dc_replace

        print(f"Loading Kokoro model (voice={voice}, provider={provider})...")
        config = PipelineConfig(voice=voice, provider=provider)
        config = dc_replace(config, generation=dc_replace(config.generation, speed=speed))
        self.pipe = KokoroPipeline(config)
        self.voice = voice
        self.speed = speed
        self.db_path = db_path
        self.last_id = get_latest_entry_id(db_path)
        self.running = True

        # Save running state
        state = load_state()
        state["running"] = True
        state["voice"] = voice
        state["speed"] = speed
        state["provider"] = provider
        save_state(state)

        print(f"Watching: {db_path}")
        print(f"Ready. Speak into Handy — your words will be spoken back.")
        print(f"Voice: {voice} | Speed: {speed}x | Provider: {provider} | Ctrl+C to quit\n")

    def speak(self, text: str):
        text = text.strip()
        if not text:
            return
        if len(text) > 500:
            text = text[:500] + "..."
            print(f"  ⚠ truncated to 500 chars")
        print(f"  ▶ {text[:80]}{'...' if len(text) > 80 else ''}")
        try:
            res = self.pipe.run(text)
            if res.audio is None or len(res.audio) == 0:
                print("  ⚠ no audio generated")
                return
            audio = self._np.array(res.audio, dtype=self._np.float32)
            duration = len(audio) / res.sample_rate
            self._sd.play(audio, res.sample_rate)
            self._sd.wait()
            print(f"  ✓ {duration:.1f}s played\n")
        except Exception as e:
            print(f"  ✗ TTS error: {e}\n")

    def run(self):
        while self.running:
            # Hot-reload config
            state = load_state()
            voice_changed = state.get("voice") != self.voice
            speed_changed = state.get("speed") != self.speed

            if voice_changed or speed_changed:
                if voice_changed:
                    print(f"  🔄 Voice changed: {self.voice} → {state['voice']}")
                    self.voice = state["voice"]
                if speed_changed:
                    print(f"  🔄 Speed changed: {self.speed} → {state.get('speed', 1.0)}")
                    self.speed = state.get("speed", 1.0)

                config = self._PipelineConfig(voice=self.voice, provider=state.get("provider", "cpu"))
                config = self._dc_replace(config, generation=self._dc_replace(config.generation, speed=self.speed))
                self.pipe = self._KokoroPipeline(config)
                print(f"  ✓ Config updated\n")

            entries = get_new_entries(self.db_path, self.last_id)
            for entry in entries:
                if not self.running:
                    break
                text = entry.get("post_processed_text") or entry.get("transcription_text", "")
                if text.strip():
                    self.speak(text.strip())
                self.last_id = entry["id"]

            time.sleep(POLL_S)

    def stop(self):
        self.running = False
        self._sd.stop()
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

    companion = ShadowCompanion(
        voice=state.get("voice", "am_michael"),
        speed=state.get("speed", 1.0),
        provider=state.get("provider", "cpu"),
        db_path=db_path,
    )

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


# ── CLI ───────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="Shadow Companion — TTS echo for language shadowing with Handy"
    )
    sub = parser.add_subparsers(dest="command")

    # Direct run (foreground)
    parser.add_argument("--voice", default="am_michael", choices=VOICE_LIST)
    parser.add_argument("--speed", type=float, default=1.0)
    parser.add_argument("--provider", default="cpu", choices=["coreml", "cpu", "auto"])
    parser.add_argument("--db", default=None)

    # Server commands
    sub.add_parser("serve", help="Start as background server")
    sub.add_parser("stop", help="Stop running server")
    sub.add_parser("status", help="Check server status")
    sub.add_parser("restart", help="Restart server")

    set_voice = sub.add_parser("set-voice", help="Change voice (hot-reloads if server running)")
    set_voice.add_argument("voice", choices=VOICE_LIST)

    set_speed = sub.add_parser("set-speed", help="Change speech speed")
    set_speed.add_argument("speed", type=float)

    # Internal
    sub.add_parser("_run_server", help=argparse.SUPPRESS)

    args = parser.parse_args()

    # Internal server command
    if args.command == "_run_server":
        _run_server()
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
