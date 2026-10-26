import {useId} from "react";

export type CommentMode = "off" | "sometimes" | "demo";
export const commentOptions: [CommentMode, string, string][] = [
  ["off", "Quiet", "Orion stays in the background. It reacts with movement only and does not talk about what it notices."],
  ["sometimes", "Lively", "Orion acts like a character. It sometimes says something about what it notices, such as a welcome or a new object. It speaks to a person once per visit, stays quiet during conversations, music and the night, and talks less when it is ignored."],
  ["demo", "Demo", "Orion reacts and talks much more often, with bigger movements, for showing Orion to people. Each comment sends a camera picture to Orion’s reply service."],
];
export const commentMode = (proactive: boolean, demo: boolean): CommentMode => demo ? "demo" : proactive ? "sometimes" : "off";
/** Demo keeps the everyday choice, so leaving Demo returns to it. */
export const commentPatch = (mode: CommentMode, proactive: boolean) =>
  mode === "demo" ? { demo_mode: true, proactive_speech: proactive }
    : { demo_mode: false, proactive_speech: mode === "sometimes" };

export function CommentSetting({proactive, demo, disabled, onChange}: {proactive: boolean; demo: boolean; disabled?: boolean;
  onChange: (patch: {demo_mode: boolean; proactive_speech: boolean}) => void}) {
  const group = useId(), value = commentMode(proactive, demo);
  return <fieldset className="dance-setting" disabled={disabled}><legend>Personality</legend>
    {commentOptions.map(([mode, label, help]) => <label key={mode}><input type="radio" name={group} value={mode} checked={value === mode}
      onChange={() => onChange(commentPatch(mode, proactive))}/><span><strong>{label}</strong><small>{help}</small></span></label>)}
  </fieldset>;
}
