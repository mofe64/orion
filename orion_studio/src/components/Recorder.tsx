import { useEffect, useMemo, useState, type PointerEvent } from "react";
import { Download, Gauge, Sparkles } from "lucide-react";
import type { GatewayConnection } from "../lib/gateway";
import { getRecorder, jointNames, median, recorderRanges, remarkName, saveSupportBundle, type Initiative, type RecorderRow, type RecorderSnapshot, type ReplySpeed } from "../lib/recorder";
import { skipReasons } from "../lib/behaviourStory";
import { FailureNotice } from "./FailureNotice";
import "./System.css";
import "./Recorder.css";

/** Categorical slots 1-5, stepped for Studio's dark surface and validated as a set. */
const SERIES_COLORS = ["#3987e5", "#d95926", "#199e70", "#c98500", "#d55181"];
const JOINTS = Object.keys(jointNames);
const W = 600, H = 170, LEFT = 44, RIGHT = 8, TOP = 10, BOTTOM = 22;

interface Series<P> { key: string; label: string; color: string; value: (row: P) => number | null }
const time = (t: number, hours: number) => new Date(t * 1000).toLocaleString([], hours > 24 ? { weekday: "short", hour: "2-digit", minute: "2-digit" } : { hour: "2-digit", minute: "2-digit" });
const nice = (value: number) => { const step = 10 ** Math.floor(Math.log10(Math.max(value, 1e-9))); return Math.ceil(value / step) * step; };

export function RecorderChart<P extends { t: number }>({ title, unit, rows, series, hours, floor }: { title: string; unit: string; rows: P[]; series: Series<P>[]; hours: number; floor?: number }) {
  const [hover, setHover] = useState<number | null>(null);
  const values = rows.flatMap(row => series.map(item => item.value(row))).filter((value): value is number => value != null && Number.isFinite(value));
  if (!rows.length || !values.length) return <figure className="recorder-chart"><figcaption>{title}</figcaption><p className="settings-help">No readings yet.</p></figure>;
  const low = floor ?? Math.min(0, ...values), high = nice(Math.max(...values, low + 1));
  const start = rows[0].t, end = Math.max(rows.at(-1)!.t, start + 1);
  const x = (t: number) => LEFT + (t - start) / (end - start) * (W - LEFT - RIGHT);
  const y = (value: number) => TOP + (1 - (value - low) / (high - low)) * (H - TOP - BOTTOM);
  const path = (item: Series<P>) => { let d = "", open = false;
    for (const row of rows) { const value = item.value(row); if (value == null) { open = false; continue; } d += `${open ? "L" : "M"}${x(row.t).toFixed(1)},${y(value).toFixed(1)} `; open = true; }
    return d; };
  const move = (event: PointerEvent<SVGSVGElement>) => {
    const box = event.currentTarget.getBoundingClientRect(), t = start + ((event.clientX - box.left) / box.width * W - LEFT) / (W - LEFT - RIGHT) * (end - start);
    let best = 0; rows.forEach((row, index) => { if (Math.abs(row.t - t) < Math.abs(rows[best].t - t)) best = index; }); setHover(best);
  };
  const picked = hover == null ? null : rows[hover];
  return <figure className="recorder-chart">
    <figcaption>{title} <span>({unit})</span></figcaption>
    {series.length > 1 && <ul className="recorder-legend">{series.map(item => <li key={item.key}><i style={{ background: item.color }} />{item.label}</li>)}</ul>}
    <div className="recorder-plot">
      <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label={`${title} over ${hours === 1 ? "the last hour" : hours === 24 ? "the last day" : "the last 7 days"}`} onPointerMove={move} onPointerLeave={() => setHover(null)}>
        {[low, (low + high) / 2, high].map(value => <g key={value}><line className="recorder-grid" x1={LEFT} x2={W - RIGHT} y1={y(value)} y2={y(value)} /><text className="recorder-axis" x={LEFT - 6} y={y(value) + 4} textAnchor="end">{Number(value.toFixed(1))}</text></g>)}
        <text className="recorder-axis" x={LEFT} y={H - 4}>{time(start, hours)}</text><text className="recorder-axis" x={W - RIGHT} y={H - 4} textAnchor="end">{time(end, hours)}</text>
        {series.map(item => <path key={item.key} d={path(item)} fill="none" stroke={item.color} strokeWidth={2} strokeLinejoin="round" vectorEffect="non-scaling-stroke" />)}
        {picked && <g><line className="recorder-crosshair" x1={x(picked.t)} x2={x(picked.t)} y1={TOP} y2={H - BOTTOM} />
          {series.map(item => { const value = item.value(picked); return value == null ? null : <circle key={item.key} cx={x(picked.t)} cy={y(value)} r={4} fill={item.color} stroke="var(--surface-1)" strokeWidth={2} />; })}</g>}
      </svg>
      {picked && <div className="recorder-tooltip" style={{ left: `${Math.min(70, x(picked.t) / W * 100)}%` }}><strong>{time(picked.t, 1)}</strong>
        {series.map(item => { const value = item.value(picked); return <span key={item.key}><i style={{ background: item.color }} />{item.label} {value == null ? "—" : `${Number(value.toFixed(1))} ${unit}`}</span>; })}</div>}
    </div>
  </figure>;
}

const jointSeries = (pick: (joint: RecorderRow["joints"][string]) => number | null): Series<RecorderRow>[] =>
  JOINTS.map((name, index) => ({ key: name, label: jointNames[name], color: SERIES_COLORS[index], value: row => row.joints[name] ? pick(row.joints[name]) : null }));
const single = <P,>(key: string, label: string, value: (row: P) => number | null): Series<P>[] => [{ key, label, color: SERIES_COLORS[0], value }];
const seconds = (value: number | null) => value == null ? "—" : `${(value / 1000).toFixed(1)} s`;

export function RecorderPanel({ connection }: { connection: GatewayConnection | null }) {
  const [hours, setHours] = useState(1);
  const [snapshot, setSnapshot] = useState<RecorderSnapshot | null>(null);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    if (!connection) return;
    const controller = new AbortController();
    setError("");
    getRecorder(connection, hours, controller.signal).then(setSnapshot)
      .catch(value => { if (!controller.signal.aborted) setError(`Recordings unavailable: ${value instanceof Error ? value.message : String(value)}`); });
    const timer = setInterval(() => setRefresh(value => value + 1), 30000);
    return () => { controller.abort(); clearInterval(timer); };
  }, [connection, hours, refresh]);
  const bundle = async () => {
    if (!connection) return;
    setSaving(true); setError("");
    try { await saveSupportBundle(connection); }
    catch (value) { setError(`Couldn’t save the support file: ${value instanceof Error ? value.message : String(value)}`); }
    finally { setSaving(false); }
  };
  const rows = snapshot?.readings ?? [];
  const latency = snapshot?.latency ?? [];
  const usual = useMemo(() => ({ total: median(latency.map(item => item.total_ms)), think: median(latency.flatMap(item => item.think_ms ?? [])),
    listen: median(latency.flatMap(item => item.listen_ms ?? [])) }), [latency]);
  return <section className="settings-card recorder"><header><Gauge size={20}/><div><h2>Recordings</h2><p>Readings, problems and reply speed that Orion keeps for 7 days.</p></div>
    <button className="quiet-button" disabled={!connection || saving} onClick={() => void bundle()}><Download size={16}/>{saving ? "Saving…" : "Save support file"}</button></header>
    <div className="recorder-ranges" role="group" aria-label="Time range">{recorderRanges.map(([value, label]) =>
      <button key={value} className="quiet-button" aria-pressed={hours === value} onClick={() => setHours(value)}>{label}</button>)}</div>
    {error && <p role="alert" className="settings-error">{error}</p>}
    <h3>Problems</h3>
    {snapshot && !snapshot.incidents.length && <p className="settings-help">No problems in this time.</p>}
    {!!snapshot?.incidents.length && <div className="system-table"><table><thead><tr><th>Time</th><th>What happened</th><th>Details</th></tr></thead><tbody>
      {snapshot.incidents.map(item => <tr key={`${item.t}-${item.message}`} className={`recorder-incident ${item.severity}`}><td>{time(item.t, 168)}</td>
        <th><span className="recorder-severity">{item.severity === "problem" ? "Problem" : item.severity === "warning" ? "Warning" : "Recovered"}</span>{item.message}</th><td>{item.detail ?? ""}</td></tr>)}
    </tbody></table></div>}
    <h3>Reply speed</h3>
    {latency.length ? <><p className="settings-help">{latency.length} replies. Usually {seconds(usual.total)} from the end of a request to Orion’s first word: {seconds(usual.listen)} to understand it, {seconds(usual.think)} to think of a reply.</p>
      <RecorderChart title="Time to first word" unit="s" hours={hours} floor={0} rows={latency}
        series={single("total", "Time to first word", (item: ReplySpeed) => item.total_ms / 1000)} /></>
      : <p className="settings-help">No replies in this time.</p>}
    <h3>Servos</h3>
    <div className="recorder-charts">
      <RecorderChart title="Servo temperature, highest" unit="°C" hours={hours} rows={rows} series={jointSeries(joint => joint.temp)} />
      <RecorderChart title="Servo current, highest" unit="mA" hours={hours} floor={0} rows={rows} series={jointSeries(joint => joint.current)} />
      <RecorderChart title="Servo power, lowest" unit="V" hours={hours} rows={rows} series={jointSeries(joint => joint.volt_min)} floor={4} />
      <RecorderChart title="Late movement steps in 10 s" unit="steps" hours={hours} floor={0} rows={rows} series={single("late", "Late steps", (row: RecorderRow) => row.loop.late)} />
      <RecorderChart title="Pi temperature, highest" unit="°C" hours={hours} rows={rows} series={single("cpu_temp", "Pi temperature", (row: RecorderRow) => row.cpu_temp)} />
    </div>
  </section>;
}

const minutes = (value: number | null | undefined) => value == null ? "—" : value < 120 ? `${Math.round(value)} s` : `${Math.round(value / 60)} min`;
const refusals: Record<string, string> = { ...skipReasons, quiet_mode: "Orion is set to Quiet.", no_detail: "Nothing to say about it." };

const clockTime = (minutes: number) => `${String(Math.floor(minutes / 60)).padStart(2, "0")}:${String(minutes % 60).padStart(2, "0")}`;
/** The owner's Lively rules in plain words. */
export function rulesInWords(rules: import("../lib/gateway").LivelyRules): string[] {
  const off = ([["wave_back", "waving back"], ["light_care", "offering light"], ["break_nudge", "break reminders"], ["goodnight", "goodnight"],
    ["follow_ups", "asking about things you mentioned"], ["new_things", "talking about new things"], ["looks", "talking when you look at it"]] as const)
    .filter(([key]) => !rules[key]).map(([, name]) => name);
  return [
    rules.quiet_from === rules.quiet_until ? "No quiet hours" : `Quiet from ${clockTime(rules.quiet_from)} to ${clockTime(rules.quiet_until)}`,
    `At most ${rules.per_hour} remarks an hour, ${rules.spacing_minutes} min apart, none for ${rules.after_talk_minutes} min after a conversation`,
    `Welcome after ${rules.welcome_after_minutes} min with nobody around; break after ${rules.break_after_minutes} min at the desk`,
    `Goodnight from ${clockTime(rules.goodnight_from)}, ${rules.sleep_after_goodnight_minutes == null ? "then stays awake" : `then sleeps after ${rules.sleep_after_goodnight_minutes} min`}`,
    off.length ? `Turned off: ${off.join(", ")}` : "Every Lively habit is on",
  ];
}

export function CharacterPanel({ initiative, ignored, quiet }: { initiative?: Initiative | null; ignored?: number; quiet?: number }) {
  if (!initiative) return <section className="settings-card"><header><Sparkles size={20}/><div><h2>Character decisions</h2></div></header><p>Character decisions are unavailable. Update Orion to see them.</p></section>;
  const { situation, curiosity, lively, quirks } = initiative;
  const facts: [string, string][] = [
    ["How lively", situation.liveliness[0].toUpperCase() + situation.liveliness.slice(1)],
    ["Quirks", `${quirks.curiosity} curiosity, ${quirks.chattiness} chattiness`],
    ["People in view", `${situation.people}${situation.looking ? ", looking at Orion" : ""}${situation.focused ? ", busy working" : ""}`],
    ["This visit", situation.visit == null ? "Nobody around" : minutes(situation.at - situation.visit)],
    ["Busy", situation.busy ? "Yes: talking, music, an alert or a routine" : "No"],
    ["Orion’s light", situation.lamp.on ? `${situation.lamp.mood.replaceAll("_", " ")} ${Math.round(situation.lamp.brightness)}%` : "Off"],
    ["Room light", situation.room_lux == null ? "Unknown" : `${Math.round(situation.room_lux)} lux`],
    ["Gestures", `${curiosity.gestures_used} of ${curiosity.gestures_allowed} in 10 min${curiosity.next_gesture_seconds > 0 ? `, next in ${minutes(curiosity.next_gesture_seconds)}` : ""}`],
    ["At the desk", minutes(lively.desk_seconds)],
    ["Room dark for", minutes(lively.dark_seconds)],
    ...(lively.goodnight_rest_seconds != null ? [["Sleeps after goodnight in", minutes(lively.goodnight_rest_seconds)] as [string, string]] : []),
    ...(ignored ? [["Ignored remarks", `${ignored}: it waits longer between remarks`] as [string, string]] : []),
    ...(quiet ? [["Asked to be quiet", `for ${minutes(quiet)} more`] as [string, string]] : []),
  ];
  return <section className="settings-card"><header><Sparkles size={20}/><div><h2>Character decisions</h2><p>What Orion sees now, and what it would say if something happened.</p></div></header>
    <dl className="debug-values">{facts.map(([key, value]) => <div key={key}><dt>{key}</dt><dd>{value}</dd></div>)}</dl>
    {initiative.rules && <><h3>Lively rules</h3><ul className="debug-rules">{rulesInWords(initiative.rules).map(line => <li key={line}>{line}</li>)}</ul></>}
    <h3>Remarks right now</h3>
    <dl className="debug-values">{Object.entries(initiative.remark_refusals).map(([event, reason]) => <div key={event}><dt>{remarkName(event)}</dt>
      <dd>{reason ? refusals[reason] ?? reason : "Would speak"}</dd></div>)}</dl>
  </section>;
}
