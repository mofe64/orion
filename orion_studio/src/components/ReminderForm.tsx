import { useState } from "react";
export const WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
export function reminderRequest(text: string, date: string, time: string, days: string[]): Record<string, unknown> {
  if (!text.trim() || text.trim().length > 200) throw new Error("Use 1–200 characters for your reminder.");
  if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(time)) throw new Error("Choose a time.");
  if (days.length) return { action: "reminder", label: text.trim(), repeat: { days, time } };
  const at = new Date(`${date}T${time}`).getTime() / 1000;
  if (!Number.isFinite(at) || at <= Date.now() / 1000) throw new Error("Choose a future date and time.");
  return { action: "reminder", label: text.trim(), due_unix: at };
}
export function ReminderForm({ disabled, onSave }: { disabled: boolean; onSave: (request: Record<string, unknown>) => Promise<boolean> }) {
  const [text, setText] = useState(""); const [date, setDate] = useState("");
  const [time, setTime] = useState("09:00"); const [days, setDays] = useState<string[]>([]);
  const [error, setError] = useState("");
  return <form className="oh-reminder-form" aria-label="New reminder" onSubmit={async event => {
    event.preventDefault(); setError("");
    try { if (await onSave(reminderRequest(text, date, time, days))) setText(""); }
    catch (value) { setError(value instanceof Error ? value.message : String(value)); }
  }}>
    <h3>New reminder</h3>
    <label>Reminder text<input required maxLength={200} value={text} disabled={disabled} onChange={event => setText(event.target.value)} /></label>
    {!days.length && <label>Date (this device’s time zone)<input type="date" required value={date} disabled={disabled} onChange={event => setDate(event.target.value)} /></label>}
    <label>Time{days.length ? " (Orion’s local time)" : ""}<input type="time" required value={time} disabled={disabled} onChange={event => setTime(event.target.value)} /></label>
    <fieldset disabled={disabled}><legend>Repeat days (optional)</legend>{WEEKDAYS.map(day => <label key={day}><input type="checkbox" checked={days.includes(day)} onChange={event => setDays(event.target.checked ? [...days, day] : days.filter(value => value !== day))} />{day}</label>)}</fieldset>
    <button type="submit" disabled={disabled}>Save reminder</button>{error && <p role="alert">{error}</p>}
  </form>;
}
