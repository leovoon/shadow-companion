import { List, ActionPanel, Action, showToast, Toast, Icon, Color } from "@raycast/api";
import { useState, useEffect } from "react";
import { runShadowCommand, getState } from "./lib";

const SPEED_OPTIONS = [
  { value: 0.5, label: "0.5x", desc: "Very slow" },
  { value: 0.75, label: "0.75x", desc: "Slow" },
  { value: 1.0, label: "1.0x", desc: "Normal" },
  { value: 1.25, label: "1.25x", desc: "Slightly fast" },
  { value: 1.5, label: "1.5x", desc: "Fast" },
  { value: 1.75, label: "1.75x", desc: "Very fast" },
  { value: 2.0, label: "2.0x", desc: "Maximum" },
];

export default function Command() {
  const [currentSpeed, setCurrentSpeed] = useState(1.0);
  const [provider, setProvider] = useState("kokoro");

  useEffect(() => {
    try {
      const state = getState();
      setCurrentSpeed(state.speed);
      setProvider(state.provider);
    } catch {
      // ignore
    }
  }, []);

  // NeuTTS does not support speed control
  if (provider === "neutts") {
    return (
      <List>
        <List.Item
          id="neutts-speed"
          title="Speed control not available"
          subtitle="NeuTTS does not support speed adjustment"
          icon={{ source: Icon.XMarkCircle, tintColor: Color.SecondaryText }}
        />
      </List>
    );
  }

  const handleSelect = async (speed: number) => {
    const toast = await showToast({ style: Toast.Style.Animated, title: `Setting speed to ${speed}x...` });
    try {
      runShadowCommand("set-speed", String(speed));
      toast.style = Toast.Style.Success;
      toast.title = `Speed set to ${speed}x`;
      setCurrentSpeed(speed);
    } catch (e) {
      toast.style = Toast.Style.Failure;
      toast.title = "Failed to set speed";
      toast.message = e instanceof Error ? e.message : String(e);
    }
  };

  return (
    <List>
      {SPEED_OPTIONS.map((option) => (
        <List.Item
          key={option.value}
          id={String(option.value)}
          title={option.label}
          subtitle={option.desc}
          icon={currentSpeed === option.value ? { source: Icon.Checkmark, tintColor: Color.Green } : Icon.Gauge}
          accessories={currentSpeed === option.value ? [{ text: "Active" }] : []}
          actions={
            <ActionPanel>
              <Action title="Select Speed" onAction={() => handleSelect(option.value)} />
            </ActionPanel>
          }
        />
      ))}
    </List>
  );
}
