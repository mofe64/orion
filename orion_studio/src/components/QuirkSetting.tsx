import { ChoiceSetting } from "./ChoiceSetting";
import type { Chattiness, Curiosity, Quirks } from "../lib/gateway";

export const quirkOptions: { [K in keyof Quirks]: { label: string; choices: [Quirks[K], string, string][] } } = {
  curiosity: { label: "Curiosity", choices: [
    ["shy", "Shy", "Looks around less and leaves things alone for longer."],
    ["normal", "Normal", "Looks around now and then."],
    ["nosy", "Nosy", "Looks around much more, comes back to things sooner and asks what they are."],
  ] },
  chattiness: { label: "Chattiness", choices: [
    ["reserved", "Reserved", "When Lively, speaks on its own half as often and keeps it to a few words."],
    ["normal", "Normal", "When Lively, speaks on its own now and then."],
    ["chatty", "Chatty", "When Lively, speaks on its own twice as often and adds a playful thought."],
  ] },
};
export const validQuirks = (value: unknown): value is Quirks => {
  const quirks = value as Quirks | null;
  return !!quirks && quirkOptions.curiosity.choices.some(([id]) => id === quirks.curiosity)
    && quirkOptions.chattiness.choices.some(([id]) => id === quirks.chattiness);
};

/** One quirk as a row of choices. */
export function QuirkSetting<K extends keyof Quirks>({ quirk, value, disabled, onChange }: { quirk: K; value: Quirks[K]; disabled?: boolean; onChange: (value: Quirks[K]) => void }) {
  const option = quirkOptions[quirk];
  return <ChoiceSetting label={option.label} options={option.choices} value={value} disabled={disabled} onChange={onChange}/>;
}
export type { Chattiness, Curiosity };
