import { List, ActionPanel, Action, showToast, Toast, Icon, Color } from "@raycast/api";
import { useState, useEffect } from "react";
import { VOICES, runShadowCommand, isRunning, getState } from "./lib";

export default function Command() {
  const [selectedVoice, setSelectedVoice] = useState<string | null>(null);
  const [provider, setProvider] = useState("kokoro");

  useEffect(() => {
    try {
      const state = getState();
      setSelectedVoice(state.voice);
      setProvider(state.provider);
    } catch {
      // ignore
    }
  }, []);

  const handleSelect = async (voiceId: string) => {
    const voice = VOICES.find((v) => v.id === voiceId);
    const toast = await showToast({ style: Toast.Style.Animated, title: `Switching to ${voice?.label || voiceId}...` });
    try {
      runShadowCommand("set-voice", voiceId);
      toast.style = Toast.Style.Success;
      toast.title = `Voice set to ${voice?.label || voiceId}`;
      toast.message = "Will take effect on next utterance";
      setSelectedVoice(voiceId);
    } catch (e) {
      toast.style = Toast.Style.Failure;
      toast.title = "Failed to switch voice";
      toast.message = e instanceof Error ? e.message : String(e);
    }
  };

  // NeuTTS uses your own cloned voice — no voice switching
  if (provider === "neutts") {
    return (
      <List>
        <List.Section title="NeuTTS Voice Cloning">
          <List.Item
            id="neutts-info"
            title="Using your cloned voice"
            subtitle="NeuTTS clones your voice from reference audio"
            icon={Icon.Person}
          />
          <List.Item
            id="neutts-setup"
            title="Re-record reference audio"
            subtitle="Run: python shadow.py setup-voice"
            icon={Icon.Microphone}
            actions={
              <ActionPanel>
                <Action
                  title="Setup Voice"
                  onAction={() => {
                    runShadowCommand("setup-voice");
                  }}
                />
              </ActionPanel>
            }
          />
        </List.Section>
      </List>
    );
  }

  // Kokoro: show full voice list
  const maleVoices = VOICES.filter((v) => v.id.startsWith("am_"));
  const femaleVoices = VOICES.filter((v) => v.id.startsWith("af_"));

  return (
    <List>
      <List.Section title="Male Voices">
        {maleVoices.map((voice) => (
          <List.Item
            key={voice.id}
            id={voice.id}
            title={voice.label}
            subtitle={voice.desc}
            icon={selectedVoice === voice.id ? { source: Icon.Checkmark, tintColor: Color.Green } : Icon.Microphone}
            accessories={selectedVoice === voice.id ? [{ text: "Active" }] : []}
            actions={
              <ActionPanel>
                <Action title="Select Voice" onAction={() => handleSelect(voice.id)} />
              </ActionPanel>
            }
          />
        ))}
      </List.Section>
      <List.Section title="Female Voices">
        {femaleVoices.map((voice) => (
          <List.Item
            key={voice.id}
            id={voice.id}
            title={voice.label}
            subtitle={voice.desc}
            icon={selectedVoice === voice.id ? { source: Icon.Checkmark, tintColor: Color.Green } : Icon.Microphone}
            accessories={selectedVoice === voice.id ? [{ text: "Active" }] : []}
            actions={
              <ActionPanel>
                <Action title="Select Voice" onAction={() => handleSelect(voice.id)} />
              </ActionPanel>
            }
          />
        ))}
      </List.Section>
    </List>
  );
}
