import { List, ActionPanel, Action, Icon, Color, showToast, Toast } from "@raycast/api";
import { useState, useEffect } from "react";
import { execSync } from "child_process";
import { existsSync } from "fs";
import path from "path";
import { isRunning, runShadowCommand, getState, getProgress } from "./lib";

const SHADOW_DIR = path.join(process.env.HOME || "~", "shadow-companion");
const METER_APP = path.join(SHADOW_DIR, "dist", "Shadow Meter.app");

const VOICE_MAP: Record<string, string> = {
  am_michael: "Michael",
  am_adam: "Adam",
  am_eric: "Eric",
  am_liam: "Liam",
  am_onyx: "Onyx",
  am_puck: "Puck",
  am_echo: "Echo",
  am_fenrir: "Fenrir",
  af_heart: "Heart",
  af_nicole: "Nicole",
  af_sarah: "Sarah",
  af_bella: "Bella",
  af_river: "River",
  af_sky: "Sky",
  af_nova: "Nova",
  af_alloy: "Alloy",
  af_aoede: "Aoede",
  af_kore: "Kore",
};

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds.toFixed(0)}s`;
  const min = Math.floor(seconds / 60);
  const sec = Math.round(seconds % 60);
  return sec > 0 ? `${min}m ${sec}s` : `${min}m`;
}

export default function Command() {
  const [running, setRunning] = useState(false);
  const [voice, setVoice] = useState("");
  const [speed, setSpeed] = useState(1.0);
  const [provider, setProvider] = useState("kokoro");
  const [progress, setProgress] = useState<{ actual: number; target: number; pct: number } | null>(null);

  useEffect(() => {
    const check = () => {
      try {
        setRunning(isRunning());
        const state = getState();
        setVoice(state.voice);
        setSpeed(state.speed);
        setProvider(state.provider);

        const p = getProgress();
        if (p) {
          setProgress({
            actual: p.actual_seconds,
            target: p.target_seconds,
            pct: Math.round(p.progress * 100),
          });
        }
      } catch {
        setRunning(false);
      }
    };
    check();
    const interval = setInterval(check, 3000);
    return () => clearInterval(interval);
  }, []);

  const handleStart = async () => {
    const toast = await showToast({ style: Toast.Style.Animated, title: "Starting Shadow Companion..." });
    try {
      runShadowCommand("serve");
      toast.style = Toast.Style.Success;
      toast.title = "🦭 Shadow Companion started";
      toast.message = `${provider === "neutts" ? "NeuTTS voice cloning" : "Kokoro streaming"} playback`;
      setRunning(true);
    } catch (e) {
      toast.style = Toast.Style.Failure;
      toast.title = "Failed to start";
      toast.message = e instanceof Error ? e.message : String(e);
    }
  };

  const handleStop = async () => {
    const toast = await showToast({ style: Toast.Style.Animated, title: "Stopping Shadow Companion..." });
    try {
      runShadowCommand("stop");
      toast.style = Toast.Style.Success;
      toast.title = "🦭 Shadow Companion stopped";
      setRunning(false);
    } catch (e) {
      toast.style = Toast.Style.Failure;
      toast.title = "Failed to stop";
      toast.message = e instanceof Error ? e.message : String(e);
    }
  };

  const meterInstalled = existsSync(METER_APP);
  const meterRunning = (() => {
    try {
      const result = execSync('pgrep -x shadow-meter', { encoding: 'utf-8' }).trim();
      return result.length > 0;
    } catch {
      return false;
    }
  })();

  const handleStartMeter = async () => {
    if (!meterInstalled) return;
    const toast = await showToast({ style: Toast.Style.Animated, title: "Starting Shadow Meter..." });
    try {
      execSync(`open "${METER_APP}"`);
      toast.style = Toast.Style.Success;
      toast.title = "📊 Shadow Meter started";
    } catch (e) {
      toast.style = Toast.Style.Failure;
      toast.title = "Failed to start Shadow Meter";
      toast.message = e instanceof Error ? e.message : String(e);
    }
  };

  const handleRestart = async () => {
    const toast = await showToast({ style: Toast.Style.Animated, title: "Restarting Shadow Companion..." });
    try {
      runShadowCommand("restart");
      toast.style = Toast.Style.Success;
      toast.title = "🦭 Shadow Companion restarted";
      setRunning(true);
    } catch (e) {
      toast.style = Toast.Style.Failure;
      toast.title = "Failed to restart";
      toast.message = e instanceof Error ? e.message : String(e);
    }
  };

  const voiceLabel = VOICE_MAP[voice] || voice;
  const providerLabel = provider === "neutts" ? "NeuTTS (voice cloning)" : "Kokoro (built-in voices)";
  const providerIcon = provider === "neutts" ? Icon.Person : Icon.Microphone;

  return (
    <List>
      <List.Section title="Status">
        <List.Item
          id="status"
          title="Server"
          accessories={[{ text: running ? "🟢 Running" : "🔴 Stopped" }]}
          icon={{ source: running ? Icon.Circle : Icon.XMarkCircle, tintColor: running ? Color.Green : Color.Red }}
        />
        <List.Item
          id="provider"
          title="Provider"
          accessories={[{ text: providerLabel }]}
          icon={providerIcon}
        />
        {provider === "kokoro" && (
          <List.Item
            id="voice"
            title="Voice"
            accessories={[{ text: voiceLabel || "am_michael" }]}
            icon={Icon.Microphone}
          />
        )}
        {provider === "kokoro" && (
          <List.Item id="speed" title="Speed" accessories={[{ text: `${speed}x` }]} icon={Icon.Gauge} />
        )}
      </List.Section>

      {progress && (
        <List.Section title="Daily Progress">
          <List.Item
            id="progress"
            title="Shadowing Time"
            accessories={[{ text: `${formatDuration(progress.actual)} / ${formatDuration(progress.target)}` }]}
            icon={Icon.Clock}
          />
          <List.Item
            id="progress-pct"
            title="Completion"
            accessories={[{ text: `${progress.pct}%` }]}
            icon={progress.pct >= 100 ? { source: Icon.Checkmark, tintColor: Color.Green } : Icon.Chart}
          />
        </List.Section>
      )}

      {meterInstalled && (
        <List.Section title="Progress Tracker">
          <List.Item
            id="meter"
            title="Shadow Meter"
            accessories={[{ text: meterRunning ? "🟢 Running" : "🔴 Stopped" }]}
            icon={Icon.Chart}
            actions={
              <ActionPanel>
                {!meterRunning && <Action title="Start Shadow Meter" onAction={handleStartMeter} />}
              </ActionPanel>
            }
          />
        </List.Section>
      )}

      <List.Section title="Actions">
        {!running ? (
          <List.Item
            id="start"
            title="Start Server"
            icon={{ source: Icon.Play, tintColor: Color.Green }}
            actions={
              <ActionPanel>
                <Action title="Start" onAction={handleStart} />
              </ActionPanel>
            }
          />
        ) : (
          <>
            <List.Item
              id="stop"
              title="Stop Server"
              icon={{ source: Icon.Stop, tintColor: Color.Red }}
              actions={
                <ActionPanel>
                  <Action title="Stop" onAction={handleStop} />
                </ActionPanel>
              }
            />
            <List.Item
              id="restart"
              title="Restart Server"
              icon={{ source: Icon.ArrowClockwise, tintColor: Color.Blue }}
              actions={
                <ActionPanel>
                  <Action title="Restart" onAction={handleRestart} />
                </ActionPanel>
              }
            />
          </>
        )}
      </List.Section>
    </List>
  );
}
