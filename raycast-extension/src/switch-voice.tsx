import { List, ActionPanel, Action, showToast, Toast, Icon, Color } from "@raycast/api";
import { useState, useEffect } from "react";
import { VOICES, runShadowCommand, isRunning, getState } from "./lib";

export default function Command() {
  const [currentVoice, setCurrentVoice] = useState("am_michael");
  const [serverRunning, setServerRunning] = useState(false);

  useEffect(() => {
    try {
      const state = getState();
      setCurrentVoice(state.voice);
    } catch {
      // defaults
    }
    setServerRunning(isRunning());
  }, []);

  const handleSelect = async (voiceId: string) => {
    const voice = VOICES.find((v) => v.id === voiceId);
    const toast = await showToast({ style: Toast.Style.Animated, title: `Switching to ${voice?.label || voiceId}...` });
    try {
      runShadowCommand("set-voice", voiceId);
      setCurrentVoice(voiceId);
      toast.style = Toast.Style.Success;
      toast.title = `🦭 Voice → ${voice?.label || voiceId}`;
      if (serverRunning) {
        toast.message = "Hot-reloads instantly via kqueue";
      }
    } catch (e) {
      toast.style = Toast.Style.Failure;
      toast.title = "Failed to switch voice";
      toast.message = e instanceof Error ? e.message : String(e);
    }
  };

  const maleVoices = VOICES.filter((v) => v.id.startsWith("am_"));
  const femaleVoices = VOICES.filter((v) => v.id.startsWith("af_"));

  return (
    <List>
      <List.Section title="Male Voices">
        {maleVoices.map((v) => (
          <List.Item
            key={v.id}
            id={v.id}
            title={v.label}
            subtitle={v.desc}
            icon={{
              source: v.id === currentVoice ? Icon.Checkmark : Icon.Circle,
              tintColor: v.id === currentVoice ? Color.Green : Color.SecondaryText,
            }}
            accessories={v.id === currentVoice ? [{ text: "Active" }] : []}
            actions={
              <ActionPanel>
                <Action title="Select Voice" onAction={() => handleSelect(v.id)} />
              </ActionPanel>
            }
          />
        ))}
      </List.Section>
      <List.Section title="Female Voices">
        {femaleVoices.map((v) => (
          <List.Item
            key={v.id}
            id={v.id}
            title={v.label}
            subtitle={v.desc}
            icon={{
              source: v.id === currentVoice ? Icon.Checkmark : Icon.Circle,
              tintColor: v.id === currentVoice ? Color.Green : Color.SecondaryText,
            }}
            accessories={v.id === currentVoice ? [{ text: "Active" }] : []}
            actions={
              <ActionPanel>
                <Action title="Select Voice" onAction={() => handleSelect(v.id)} />
              </ActionPanel>
            }
          />
        ))}
      </List.Section>
    </List>
  );
}
