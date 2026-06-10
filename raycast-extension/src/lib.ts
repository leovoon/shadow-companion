import { execSync } from "child_process";
import { existsSync, readFileSync } from "fs";
import path from "path";

const SHADOW_DIR = path.join(process.env.HOME || "~", "shadow-companion");
const VENV_PYTHON = path.join(SHADOW_DIR, ".venv", "bin", "python3");
const SHADOW_SCRIPT = path.join(SHADOW_DIR, "shadow.py");
const STATE_FILE = path.join(process.env.HOME || "~", ".shadow-companion", "state.json");
const DAILY_PROGRESS_FILE = path.join(process.env.HOME || "~", ".shadow-companion", "daily-progress.json");

function python(): string {
  try {
    execSync(`test -f "${VENV_PYTHON}"`);
    return VENV_PYTHON;
  } catch {
    return "python3";
  }
}

export function runShadowCommand(...args: string[]): string {
  const cmd = `"${python()}" "${SHADOW_SCRIPT}" ${args.join(" ")}`;
  try {
    return execSync(cmd, { encoding: "utf-8", timeout: 10000 }).trim();
  } catch (e: unknown) {
    const error = e as { stderr?: string; message?: string };
    throw new Error(error.stderr?.trim() || error.message || "Command failed");
  }
}

export interface CompanionState {
  voice: string;
  speed: number;
  provider: string;
  running: boolean;
}

export interface DailyProgress {
  date: string;
  actual_seconds: number;
  stt_seconds: number;
  target_seconds: number;
  progress: number;
}

export function getState(): CompanionState {
  try {
    const raw = readFileSync(STATE_FILE, "utf-8");
    return JSON.parse(raw);
  } catch {
    return { voice: "am_michael", speed: 1.0, provider: "kokoro", running: false };
  }
}

export function getProgress(): DailyProgress | null {
  try {
    const raw = readFileSync(DAILY_PROGRESS_FILE, "utf-8");
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

export function isRunning(): boolean {
  try {
    const pidPath = path.join(process.env.HOME || "~", ".shadow-companion", "server.pid");
    if (!existsSync(pidPath)) return false;
    const pid = parseInt(readFileSync(pidPath, "utf-8").trim(), 10);
    if (isNaN(pid)) return false;
    // signal 0 throws if PID doesn't exist
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

export const VOICES = [
  { id: "am_michael", label: "Michael", desc: "American male, clear" },
  { id: "am_adam", label: "Adam", desc: "American male, warm" },
  { id: "am_eric", label: "Eric", desc: "American male, deep" },
  { id: "am_liam", label: "Liam", desc: "American male, young" },
  { id: "am_onyx", label: "Onyx", desc: "American male, rich" },
  { id: "am_puck", label: "Puck", desc: "American male, playful" },
  { id: "am_echo", label: "Echo", desc: "American male, resonant" },
  { id: "am_fenrir", label: "Fenrir", desc: "American male, deep" },
  { id: "af_heart", label: "Heart", desc: "American female, warm" },
  { id: "af_nicole", label: "Nicole", desc: "American female, clear" },
  { id: "af_sarah", label: "Sarah", desc: "American female, natural" },
  { id: "af_bella", label: "Bella", desc: "American female, soft" },
  { id: "af_river", label: "River", desc: "American female, calm" },
  { id: "af_sky", label: "Sky", desc: "American female, bright" },
  { id: "af_nova", label: "Nova", desc: "American female, bright" },
  { id: "af_alloy", label: "Alloy", desc: "American female, neutral" },
  { id: "af_aoede", label: "Aoede", desc: "American female, melodic" },
  { id: "af_kore", label: "Kore", desc: "American female, gentle" },
];
