import { useId } from "react";
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
  const group = useId(), option = quirkOptions[quirk];
  return <fieldset className="dance-setting" disabled={disabled}><legend>{option.label}</legend>
    {option.choices.map(([id, label, help]) => <label key={id}><input type="radio" name={group} value={id} checked={value === id}
      onChange={() => onChange(id)}/><span><strong>{label}</strong><small>{help}</small></span></label>)}
  </fieldset>;
}
export type { Chattiness, Curiosity };
