export interface StudioPreferences { debugMode: boolean; reduceMotion: boolean; previewAudio: boolean }
export const DEFAULT_PREFERENCES: StudioPreferences = { debugMode: false, reduceMotion: false, previewAudio: true };
const KEY = "orion-studio:preferences:v1";
export function loadPreferences(): StudioPreferences {
  try {
    const stored = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    return Object.fromEntries(Object.entries(DEFAULT_PREFERENCES).map(([key,value]) => [key, typeof stored?.[key] === "boolean" ? stored[key] : value])) as unknown as StudioPreferences;
  } catch { return { ...DEFAULT_PREFERENCES }; }
}
export function savePreferences(value: StudioPreferences) { localStorage.setItem(KEY, JSON.stringify(value)); }
