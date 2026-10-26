import { ChoiceSetting } from "./ChoiceSetting";

export type CommentMode = "off" | "sometimes" | "demo";
export const commentOptions: [CommentMode, string, string][] = [
  ["off", "Quiet", "Orion stays in the background. It reacts with movement only and does not talk about what it notices."],
  ["sometimes", "Lively", "Orion acts like a character. It waves back, welcomes you, offers light when the room gets dark, suggests a break after a long time at the desk and says goodnight when you leave late. It speaks to a person once per visit, stays quiet during conversations, music and the night, and talks less when it is ignored."],
  ["demo", "Demo", "Orion reacts and talks much more often, with bigger movements, for showing Orion to people. Each comment sends a camera picture to Orion’s reply service."],
];
export const commentMode = (proactive: boolean, demo: boolean): CommentMode => demo ? "demo" : proactive ? "sometimes" : "off";
/** Demo keeps the everyday choice, so leaving Demo returns to it. */
export const commentPatch = (mode: CommentMode, proactive: boolean) =>
  mode === "demo" ? { demo_mode: true, proactive_speech: proactive }
    : { demo_mode: false, proactive_speech: mode === "sometimes" };

export function CommentSetting({proactive, demo, disabled, onChange}: {proactive: boolean; demo: boolean; disabled?: boolean;
  onChange: (patch: {demo_mode: boolean; proactive_speech: boolean}) => void}) {
  return <ChoiceSetting label="How lively Orion is" options={commentOptions} value={commentMode(proactive, demo)} disabled={disabled}
    onChange={mode => onChange(commentPatch(mode, proactive))}/>;
}
