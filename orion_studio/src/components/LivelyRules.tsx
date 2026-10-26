import { useEffect, useState } from "react";
import type { LivelyRules as Rules } from "../lib/gateway";
import "./LivelyRules.css";

export const usualRules: Rules = {
  quiet_from: 22 * 60, quiet_until: 7 * 60, per_hour: 4, spacing_minutes: 10, after_talk_minutes: 5, welcome_after_minutes: 10,
  break_after_minutes: 90, goodnight_from: 21 * 60, sleep_after_goodnight_minutes: 2,
  wave_back: true, light_care: true, break_nudge: true, goodnight: true, follow_ups: true, new_things: true, looks: true,
};
type Switch = "wave_back" | "light_care" | "break_nudge" | "goodnight" | "follow_ups" | "new_things" | "looks";
export const ruleSwitches: [Switch, string, string][] = [
  ["wave_back", "Wave back", "Waves back when you wave at it."],
  ["light_care", "Offer light", "Offers to turn its light on or up when the room gets dark."],
  ["break_nudge", "Suggest a break", "Stretches and suggests a break after a long time at the desk."],
  ["goodnight", "Say goodnight", "Says goodnight when you leave late, then goes to sleep."],
  ["follow_ups", "Ask about things you mentioned", "Remembers an interview or a trip you talk about, and asks how it went."],
  ["new_things", "Talk about new things", "Says something about a new thing it sees on the desk."],
  ["looks", "Talk when you look at it", "Says something when you look at it for a while."],
];
const clock = (minutes: number) => `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
const minutesOf = (value: string) => { const [hours, minutes] = value.split(":").map(Number); return Number.isInteger(hours) && Number.isInteger(minutes) ? hours * 60 + minutes : null; };

function Stepper({ label, value, min, max, step = 1, unit, onChange, disabled }: { label: string; value: number; min: number; max: number; step?: number; unit: string; onChange: (value: number) => void; disabled?: boolean }) {
  const set = (next: number) => onChange(Math.min(max, Math.max(min, next)));
  return <span className="rule-stepper" role="group" aria-label={label}>
    <button type="button" aria-label={`Less: ${label}`} disabled={disabled || value <= min} onClick={() => set(value - step)}>−</button>
    <output aria-live="polite">{value} {unit}</output>
    <button type="button" aria-label={`More: ${label}`} disabled={disabled || value >= max} onClick={() => set(value + step)}>+</button>
  </span>;
}
function Time({ label, value, min, onChange, disabled }: { label: string; value: number; min?: number; onChange: (value: number) => void; disabled?: boolean }) {
  return <input className="rule-time" type="time" aria-label={label} step={900} value={clock(value)} disabled={disabled}
    min={min === undefined ? undefined : clock(min)}
    onChange={event => { const next = minutesOf(event.target.value); if (next !== null && (min === undefined || next >= min)) onChange(next); }} />;
}

/** Lively's rules as sentences the owner can change. Changes save shortly after the last tap. */
export function LivelyRules({ rules, pending, onChange }: { rules: Rules; pending: boolean; onChange: (change: Partial<Rules>) => void }) {
  const [draft, setDraft] = useState<Partial<Rules>>({});
  const shown = { ...rules, ...draft };
  const change = (patch: Partial<Rules>) => setDraft(old => ({ ...old, ...patch }));
  useEffect(() => {
    if (!Object.keys(draft).length || pending) return;
    const timer = setTimeout(() => { onChange(draft); setDraft({}); }, 600);
    return () => clearTimeout(timer);
  }, [draft, pending, onChange]);
  const usual = (Object.keys(usualRules) as (keyof Rules)[]).every(key => shown[key] === usualRules[key]);
  const never = shown.sleep_after_goodnight_minutes === null;
  return <details className="lively-rules">
    <summary>Fine-tune Lively</summary>
    <p className="settings-help">These rules apply when Orion is Lively. Demo uses its own, faster timings. Chattiness changes how often: a reserved Orion talks about half as often, a chatty one more often.</p>
    <h4>When Orion talks on its own</h4>
    <ul className="rule-list">
      <li><span>Stay quiet from</span><Time label="Quiet from" value={shown.quiet_from} onChange={value => change({ quiet_from: value })} /><span>to</span><Time label="Quiet until" value={shown.quiet_until} onChange={value => change({ quiet_until: value })} /></li>
      <li><span>Say at most</span><Stepper label="Remarks each hour" value={shown.per_hour} min={1} max={12} unit={shown.per_hour === 1 ? "thing" : "things"} onChange={value => change({ per_hour: value })} /><span>on its own each hour</span></li>
      <li><span>Wait at least</span><Stepper label="Minutes between remarks" value={shown.spacing_minutes} min={1} max={120} step={shown.spacing_minutes >= 20 ? 5 : 1} unit="min" onChange={value => change({ spacing_minutes: value })} /><span>between two remarks</span></li>
      <li><span>Stay quiet for</span><Stepper label="Minutes after a conversation" value={shown.after_talk_minutes} min={0} max={60} unit="min" onChange={value => change({ after_talk_minutes: value })} /><span>after you talk to it</span></li>
      <li><span>Welcome you when the room was empty for</span><Stepper label="Empty room before a welcome" value={shown.welcome_after_minutes} min={2} max={240} step={shown.welcome_after_minutes >= 30 ? 10 : 2} unit="min" onChange={value => change({ welcome_after_minutes: value })} /></li>
    </ul>
    <h4>Looking after you</h4>
    <ul className="rule-list">
      <li><span>Suggest a break after</span><Stepper label="Minutes at the desk before a break" value={shown.break_after_minutes} min={20} max={240} step={10} unit="min" onChange={value => change({ break_after_minutes: value })} /><span>at the desk</span></li>
      <li><span>Say goodnight to people who leave after</span><Time label="Goodnight from" value={shown.goodnight_from} min={18 * 60} onChange={value => change({ goodnight_from: value })} /></li>
      <li>{never ? <span>After a goodnight, stay awake</span> : <><span>After a goodnight, go to sleep after</span><Stepper label="Minutes before sleep after a goodnight" value={shown.sleep_after_goodnight_minutes ?? 2} min={1} max={60} unit="min" onChange={value => change({ sleep_after_goodnight_minutes: value })} /></>}
        <button type="button" className="quiet-button rule-link" onClick={() => change({ sleep_after_goodnight_minutes: never ? 2 : null })}>{never ? "Go to sleep instead" : "Stay awake instead"}</button></li>
    </ul>
    <h4>What Orion does</h4>
    <div className="rule-switches">{ruleSwitches.map(([key, label, help]) => <div className="settings-row" key={key}><div><strong>{label}</strong><p>{help}</p></div>
      <button type="button" className="studio-switch" role="switch" aria-label={label} aria-checked={shown[key]} onClick={() => change({ [key]: !shown[key] })}><span /></button></div>)}</div>
    <div className="rule-footer"><button type="button" className="quiet-button" disabled={usual} onClick={() => change({ ...usualRules })}>Use the usual rules</button>
      <span className="settings-help" role="status">{Object.keys(draft).length || pending ? "Saving…" : usual ? "Using the usual rules." : "Saved on Orion."}</span></div>
  </details>;
}
