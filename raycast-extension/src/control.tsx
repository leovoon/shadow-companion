import { List, ActionPanel, Action, Icon, Color, showToast, Toast } from "@raycast/api";
import { useState, useEffect } from "react";
import { isRunning, runShadowCommand, getState } from "./lib";

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

export default function Command() {
  const [running, setRunning] = useState(false);
  const [voice, setVoice] = useState("");
  const [speed, setSpeed] = useState(1.0);

  useEffect(() => {
    const check = () => {
      try {
        setRunning(isRunning());
        const state = getState();
        setVoice(state.voice);
        setSpeed(state.speed);
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
      toast.message = "kqueue watching · streaming playback";
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
          id="voice"
          title="Voice"
          accessories={[{ text: voiceLabel || "am_michael" }]}
          icon={Icon.Microphone}
        />
        <List.Item id="speed" title="Speed" accessories={[{ text: `${speed}x` }]} icon={Icon.Gauge} />
      </List.Section>

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
