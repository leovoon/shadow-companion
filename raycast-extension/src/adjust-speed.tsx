import { List, ActionPanel, Action, showToast, Toast, Icon, Color } from "@raycast/api";
import { useState, useEffect } from "react";
import { runShadowCommand, getState } from "./lib";

const SPEED_OPTIONS = [
  { value: 0.7, label: "0.7x — Very Slow", desc: "For careful pronunciation practice" },
  { value: 0.8, label: "0.8x — Slow", desc: "Good for shadowing beginners" },
  { value: 0.85, label: "0.85x — Slightly Slow", desc: "Comfortable shadowing pace" },
  { value: 0.9, label: "0.9x — Near Normal", desc: "Slightly easier to follow" },
  { value: 1.0, label: "1.0x — Normal", desc: "Native speaking speed" },
  { value: 1.1, label: "1.1x — Slightly Fast", desc: "Push your listening speed" },
  { value: 1.2, label: "1.2x — Fast", desc: "Challenge mode" },
  { value: 1.3, label: "1.3x — Very Fast", desc: "Advanced shadowing" },
];

export default function Command() {
  const [currentSpeed, setCurrentSpeed] = useState(1.0);

  useEffect(() => {
    try {
      const state = getState();
      setCurrentSpeed(state.speed);
    } catch {
      // default
    }
  }, []);

  const handleSelect = async (speed: number) => {
    const toast = await showToast({ style: Toast.Style.Animated, title: `Setting speed to ${speed}x...` });
    try {
      runShadowCommand("set-speed", String(speed));
      setCurrentSpeed(speed);
      toast.style = Toast.Style.Success;
      toast.title = `Speed set to ${speed}x`;
    } catch (e) {
      toast.style = Toast.Style.Failure;
      toast.title = "Failed to set speed";
      toast.message = e instanceof Error ? e.message : String(e);
    }
  };

  return (
    <List>
      {SPEED_OPTIONS.map((opt) => (
        <List.Item
          key={opt.value}
          id={String(opt.value)}
          title={opt.label}
          subtitle={opt.desc}
          icon={{
            source: opt.value === currentSpeed ? Icon.Checkmark : Icon.Circle,
            tintColor: opt.value === currentSpeed ? Color.Green : Color.SecondaryText,
          }}
          accessories={opt.value === currentSpeed ? [{ text: "Active" }] : []}
          actions={
            <ActionPanel>
              <Action title="Set Speed" onAction={() => handleSelect(opt.value)} />
            </ActionPanel>
          }
        />
      ))}
    </List>
  );
}
