import { List, ActionPanel, Action, showToast, Toast, Icon, Color } from "@raycast/api";
import { useState, useEffect } from "react";
import { runShadowCommand, getState, isRunning } from "./lib";

const PROVIDERS = [
  {
    id: "kokoro",
    label: "Kokoro",
    desc: "Built-in voices, adjustable speed",
    icon: Icon.Microphone,
  },
  {
    id: "neutts",
    label: "NeuTTS Air",
    desc: "Voice cloning from your reference audio",
    icon: Icon.Person,
  },
];

export default function Command() {
  const [currentProvider, setCurrentProvider] = useState("kokoro");
  const [running, setRunning] = useState(false);

  useEffect(() => {
    try {
      const state = getState();
      setCurrentProvider(state.provider);
      setRunning(isRunning());
    } catch {
      // ignore
    }
  }, []);

  const handleSelect = async (providerId: string) => {
    const provider = PROVIDERS.find((p) => p.id === providerId);
    const toast = await showToast({
      style: Toast.Style.Animated,
      title: `Switching to ${provider?.label || providerId}...`,
    });
    try {
      runShadowCommand("set-provider", providerId);
      toast.style = Toast.Style.Success;
      toast.title = `Provider set to ${provider?.label || providerId}`;
      toast.message = "Restart required to take effect";
      setCurrentProvider(providerId);
    } catch (e) {
      toast.style = Toast.Style.Failure;
      toast.title = "Failed to switch provider";
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

  return (
    <List>
      <List.Section title="TTS Provider">
        {PROVIDERS.map((provider) => (
          <List.Item
            key={provider.id}
            id={provider.id}
            title={provider.label}
            subtitle={provider.desc}
            icon={
              currentProvider === provider.id
                ? { source: Icon.Checkmark, tintColor: Color.Green }
                : provider.icon
            }
            accessories={currentProvider === provider.id ? [{ text: "Active" }] : []}
            actions={
              <ActionPanel>
                <Action title="Select Provider" onAction={() => handleSelect(provider.id)} />
                {currentProvider !== provider.id && running && (
                  <Action title="Restart to Apply" onAction={handleRestart} />
                )}
              </ActionPanel>
            }
          />
        ))}
      </List.Section>

      {currentProvider === "neutts" && (
        <List.Section title="NeuTTS Setup">
          <List.Item
            id="setup-voice"
            title="Record Reference Audio"
            subtitle="python shadow.py setup-voice"
            icon={Icon.Microphone}
            actions={
              <ActionPanel>
                <Action title="Setup Voice" onAction={() => runShadowCommand("setup-voice")} />
              </ActionPanel>
            }
          />
        </List.Section>
      )}
    </List>
  );
}
