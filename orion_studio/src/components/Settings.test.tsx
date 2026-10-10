import { changeCharacterMode, CHARACTER_MODE_FAILURE } from "./Home";
import { describe, expect, it, vi } from "vitest";
import { canSaveVoiceSettings } from "./Settings";
import type { StudioVoice } from "../hooks/useStudioVoice";
import { DEFAULT_VOICE_SETTINGS } from "../lib/studioVoicePipeline";
import { renderToStaticMarkup } from "react-dom/server";
import { Settings } from "./Settings";
import { DEFAULT_PREFERENCES } from "../lib/preferences";

const voice = { settings: DEFAULT_VOICE_SETTINGS, models: [], loaded: true, saving: false,
  snapshot: { muted: false } } as unknown as StudioVoice;

describe("voice settings save availability", () => {
  it("allows a speech change while listening even before the model list loads", () => {
    const draft = { ...DEFAULT_VOICE_SETTINGS, asrModel: "another-asr" };
    expect(canSaveVoiceSettings(voice, draft, "codex", true)).toBe(true);
  });
  it("requires a change and a connected Pi", () => {
    expect(canSaveVoiceSettings(voice, DEFAULT_VOICE_SETTINGS, "codex", true)).toBe(false);
    expect(canSaveVoiceSettings(voice, { ...DEFAULT_VOICE_SETTINGS, asrModel: "another-asr" }, "codex", false)).toBe(false);
  });
});

function markup(value: StudioVoice, desktop = true, connected = true) {
  return renderToStaticMarkup(<Settings desktop={desktop} voice={value} preferences={DEFAULT_PREFERENCES} onPreferences={() => {}} theme="dark" onTheme={() => {}} status={null} connected={connected} onConnect={() => {}} onHome={() => {}} onSound={async () => {}} />);
}
describe("browser development settings", () => {
  it.each([true, false])("explains the desktop prerequisite with connection=%s", connected => {
    const html = markup({ ...voice, loaded: false, loadError: { message: "Studio couldn't load Orion's voice settings.", detail: "missing native bridge" } }, false, connected);
    expect(html).toContain("Open Orion Studio’s desktop app and connect Orion to view or change voice settings.");
    expect(html).toContain("Open Orion Studio’s desktop app and connect Orion to view or change personality and memories.");
    expect(html).not.toContain("couldn&#x27;t load");
    expect(html).not.toContain("Technical details");
    expect(html).not.toContain("Try again");
    expect(html).not.toContain("Save voice settings");
    expect(html).not.toContain("· saved");
  });
});
describe("Pi speech settings", () => {
  it("shows a loading state instead of inferring local speech from defaults", () => {
    const html = markup({ ...voice, loaded: false, loadError: null });
    expect(html).toContain("Loading Orion’s voice settings…");
    expect(html).not.toContain("Choose folder");
    expect(html).not.toContain("Apple Silicon");
    expect(html).not.toContain("Chatterbox");
    expect(html).not.toContain("Voice: Piper");
    const agent = html.slice(html.indexOf("<h2>Voice</h2>"), html.indexOf("<h2>Studio</h2>"));
    expect(agent).toContain("Loading Orion’s voice settings…");
    expect(agent).not.toContain("<select");
    expect(agent).not.toContain("· saved");
  });
  it("shows a plain load failure with collapsed detail, never the local model layout", () => {
    const html = markup({ ...voice, loaded: false, loadError: { message: "Studio couldn't load Orion's voice settings.", detail: "native failure" } });
    expect(html).toContain("Studio couldn&#x27;t load Orion&#x27;s voice settings.");
    expect(html).toContain("Technical details");
    expect(html).not.toContain("Choose folder");
    expect(html).not.toContain("Local models");
    expect(html).not.toContain("<details open");
    const agent = html.slice(html.indexOf("<h2>Voice</h2>"), html.indexOf("<h2>Studio</h2>"));
    expect(agent).toContain("Studio couldn&#x27;t load Orion&#x27;s voice settings.");
    expect(agent).toContain("Technical details");
    expect(agent).not.toContain("<select");
    expect(agent).not.toContain("· saved");
  });
  it("uses Pi ownership even when loaded settings have an old model ID", () => {
    const html = markup(voice);
    expect(html).toContain("Speech recognition and voices run on Orion.");
    expect(html).toContain("Voice: Alba (British English)");
    expect(html).not.toContain("Rustpotter");
    expect(html).not.toContain("Silero VAD");
    expect(html).not.toContain("Qwen3");
    expect(html).not.toContain("Coming soon");
    expect(html).toContain("Change mode on Home");
    expect(html).not.toContain('aria-label="Character mode"');
    expect(html).not.toContain("Stored on this computer.");
    expect(html).toContain("Saved reply model · saved");
    expect(html).toContain("Medium · saved");
  });
});

describe("character mode failures", () => {
  it.each([true, false])("maps a failure while preserving the requested enabled value %s", async enabled => {
    const change = vi.fn(async () => { throw new Error("HTTP runtime-stack: rejected"); });
    await expect(changeCharacterMode(change, enabled)).rejects.toMatchObject({ message: CHARACTER_MODE_FAILURE, detail: "HTTP runtime-stack: rejected" });
    expect(change).toHaveBeenCalledWith(enabled);
  });
});

it("preserves saved raw IDs while displaying friendly model and effort names", () => {
  const html = markup({ ...voice, models: [{ model: DEFAULT_VOICE_SETTINGS.model, name: "GPT-6 Luna", efforts: ["low", "medium", "high"] }] });
  expect(html).toContain(`value="${DEFAULT_VOICE_SETTINGS.model}" selected="">GPT-6 Luna</option>`);
  expect(html).toContain('value="medium" selected="">Medium</option>');
  expect(html).not.toContain("Active agent:");
  expect(html).not.toContain("API key");
  expect(html).toContain("CC BY 4.0");
});

it("orders owner settings and keeps the sole Save action beside its restart consequence", () => {
  const html = markup(voice);
  const headings = [...html.matchAll(/<h2>(.*?)<\/h2>/g)].map(match => match[1]);
  expect(headings).toEqual(["Orion", "Personality and memory", "Voice", "Studio", "Developer tools"]);
  expect(html.match(/>Save [^<]+<\/button>/g)).toEqual([">Save voice settings</button>"]);
  const save = html.slice(html.indexOf('class="settings-save"'),html.indexOf("<h2>Studio"));
  expect(save).toContain("restarts the voice coordinator");
  expect(save).toContain("turns listening off");
});

it("shows a pairing prerequisite in the disconnected desktop app", () => {
  const html = markup({...voice,loaded:false},true,false);
  expect(html).toContain("Connect Orion to view or change voice settings.");
  expect(html).toContain("Connect Orion to view or change personality and memories.");
  expect(html).not.toContain("Loading Orion’s voice settings");
  expect(html).not.toContain("Loading personality");
});
