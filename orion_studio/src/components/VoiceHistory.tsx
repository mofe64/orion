import { isTauri, invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import "./VoiceHistory.css";
import { failure, forOwner, type Failure } from "../lib/feedback";
import { FailureNotice } from "./FailureNotice";

interface Summary {
  sessionId: string;
  startedAt: string;
  updatedAt: string;
  command: string | null;
  reply: string | null;
  error: string | null;
}
interface Page { items: Summary[]; nextCursor: string | null }
interface Entry { at: string; event: Record<string, unknown> }
interface Turn { sessionId: string; startedAt: string; updatedAt: string; events: Entry[] }

export function loadHistory(sessionId: string | null, before: string | null) {
  const message = sessionId ? "Studio couldn't open this conversation. Please try again." : before ? "Studio couldn't load older conversations. Please try again." : "Studio couldn't load Orion's conversation history. Please try again.";
  return forOwner(() => invoke<Page | Turn>("load_voice_history", { sessionId, before }), message);
}

const time = (date: string) => new Date(date).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit" });
const day = (date: string) => new Date(date).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long", year: "numeric" });
const ms = (value: unknown) => typeof value === "number" && Number.isFinite(value) ? value >= 1000 ? `${(value / 1000).toFixed(2)} s` : `${Math.round(value)} ms` : "—";
const str = (value: unknown) => typeof value === "string" ? value : "";
const readable = (value: unknown) => JSON.stringify(value, null, 2) ?? "—";
const toolName = (name: string) => ({ set_timer: "Set timer", set_alarm: "Set alarm", list_alerts: "Check alarms and timers", cancel_alert: "Cancel alert", stop_alert: "Stop alert", set_lighting: "Change light", set_mode: "Change mode", go_to_sleep: "Go to sleep", search_memories: "Search memories", append_memory: "Save memory" } as Record<string,string>)[name] ?? name.replaceAll("_", " ");

function title(event: Record<string, unknown>): string | null {
  switch (event.type) {
    case "transcription.started": return "Listening ended; turning speech into text";
    case "transcript.final": return "You said";
    case "agent.started": return "Sent to the agent";
    case "agent.progress": return str(event.message) || "Agent is working";
    case "agent.tool": return toolName(str(event.name));
    case "agent.response": return "Orion replied";
    case "synthesis.started": return "Creating Orion’s voice";
    case "speech.started": return "Playback started";
    case "speech.completed": return "Playback finished";
    case "worker.error": return "Something went wrong";
    case "stage.timing": return ({ transcription_wake_prefix: "Wake check", transcription_wake_and_command: "Speech to text", transcription_command: "Speech to text", synthesisTotalMs: "Audio generation", firstChunkMs: "First audio ready", firstPlaybackMs: "Audio to playback" } as Record<string,string>)[str(event.stage)] ?? null;
    default: return null;
  }
}

export function VoiceHistory({ onBack, connected, desktop = isTauri() }: { onBack: () => void; connected: boolean; desktop?: boolean }) {
  const available = connected && desktop;
  const [turns, setTurns] = useState<Summary[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [turn, setTurn] = useState<Turn | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Failure | null>(null);
  const detailRequest = useRef(0);
  const connectionEpoch = useRef(0);
  const loadTurn = async (sessionId: string) => {
    const request = ++detailRequest.current;
    setSelected(sessionId); setTurn(null); setError(null);
    try { const result = await loadHistory(sessionId, null) as Turn; if (request === detailRequest.current) setTurn(result); }
    catch (reason) { if (request === detailRequest.current) setError(failure("Studio couldn't open this conversation. Please try again.", reason)); }
  };
  const refresh = async () => {
    const epoch = connectionEpoch.current;
    setLoading(true); setError(null);
    try {
      const result = await loadHistory(null, null) as Page;
      if (epoch !== connectionEpoch.current) return;
      setTurns(result.items); setNextCursor(result.nextCursor);
      if (result.items.length) await loadTurn(selected && result.items.some(item => item.sessionId === selected) ? selected : result.items[0].sessionId);
      if (!result.items.length) { setSelected(null); setTurn(null); }
    } catch (reason) { if (epoch === connectionEpoch.current) setError(failure("Studio couldn't load Orion's conversation history. Please try again.", reason)); }
    finally { if (epoch === connectionEpoch.current) setLoading(false); }
  };
  const older = async () => {
    if (!nextCursor) return;
    const epoch = connectionEpoch.current;
    setLoading(true); setError(null);
    try { const page = await loadHistory(null, nextCursor) as Page; if (epoch === connectionEpoch.current) { setTurns(previous => [...previous, ...page.items]); setNextCursor(page.nextCursor); } }
    catch (reason) { if (epoch === connectionEpoch.current) setError(failure("Studio couldn't load older conversations. Please try again.", reason)); }
    finally { if (epoch === connectionEpoch.current) setLoading(false); }
  };
  useEffect(() => {
    connectionEpoch.current++; detailRequest.current++; setError(null);
    if (available) void refresh(); else { setLoading(false); setTurns([]); setNextCursor(null); setSelected(null); setTurn(null); }
    return () => { connectionEpoch.current++; detailRequest.current++; };
  }, [available]);
  const visible = turn?.events.map((entry, index) => ({ entry, index, label: title(entry.event) })).filter(item => item.label) ?? [];
  let previousDay = "";
  const elapsed = turn ? new Date(turn.updatedAt).getTime() - new Date(turn.startedAt).getTime() : 0;
  return <section className="settings-page voice-history" aria-labelledby="voice-history-title">
    <header className="voice-history-heading"><div><button className="quiet-button" type="button" onClick={onBack}>← Back to Home</button><h1 id="voice-history-title">History</h1><p>Saved on Orion. Times show what happened during each voice request.</p></div><button className="quiet-button" type="button" disabled={!available || loading} onClick={() => void refresh()}>{loading ? "Loading…" : "Refresh"}</button></header>
    {!desktop ? <p role="status" className="settings-help">Open Orion Studio’s desktop app and connect Orion to read conversation history.</p> : !connected && <p role="status" className="settings-help">Connect Orion to read its conversation history.</p>}
    <FailureNotice value={error} />
    <div className="voice-history-layout"><aside className="voice-history-list" aria-label="Conversations">
      {!turns.length && <p className="voice-history-empty">{!available ? "Saved conversations are available after connecting in the desktop app." : loading ? "Reading Orion’s history…" : "No saved conversations yet. New voice requests will appear here."}</p>}
      {turns.map(item => { const heading = day(item.startedAt); const showDay = heading !== previousDay; previousDay = heading; return <div key={item.sessionId}>{showDay && <h2>{heading}</h2>}<button type="button" className={item.sessionId === selected ? "voice-history-item selected" : "voice-history-item"} aria-current={item.sessionId === selected ? "true" : undefined} onClick={() => void loadTurn(item.sessionId)}><time dateTime={item.startedAt}>{time(item.startedAt)}</time><strong>{item.command || "Voice request"}</strong><span>{item.error ? `Needs attention · ${item.error}` : item.reply ? item.reply : "No reply recorded"}</span></button></div>; })}
      {nextCursor && <button className="quiet-button voice-history-older" type="button" disabled={loading} onClick={() => void older()}>{loading ? "Loading…" : "Load older"}</button>}
    </aside><article className="voice-history-detail">
      {turn ? <><header><p className="eyebrow">{day(turn.startedAt)} · {time(turn.startedAt)}</p><h2>{turn.events.find(entry => entry.event.type === "transcript.final")?.event.text as string || "Voice request"}</h2><p>Elapsed {ms(elapsed)} · {visible.length} steps</p></header><ol className="voice-history-timeline">{visible.map(({entry,index,label}) => { const event = entry.event; return <li key={index}><div className="voice-history-step"><time dateTime={entry.at}>{time(entry.at)} <span>+{ms(new Date(entry.at).getTime() - new Date(turn.startedAt).getTime())}</span></time><h3>{label}</h3>{event.type === "transcript.final" && <p>{str(event.text)}</p>}{event.type === "agent.response" && <p>{str(event.text)}</p>}{event.type === "worker.error" && <p className="settings-error">{str(event.message)}</p>}{event.type === "agent.tool" && <><p>{event.success ? "Completed" : "Failed"} · {ms(event.durationMs)}</p><details><summary>Request and result</summary><div><strong>Request</strong><pre>{readable(event.arguments)}</pre><strong>Result</strong><pre>{readable(event.result)}</pre></div></details></>}{event.type === "stage.timing" && <p>{ms(event.durationMs)}</p>}{event.type === "agent.response" && <small>Agent time {ms(event.durationMs)}</small>}</div></li>; })}</ol></> : <div className="voice-history-empty">{!available ? "Connect in the desktop app to view the steps." : selected ? "Opening conversation…" : "Choose a conversation to see its steps and times."}</div>}
    </article></div>
  </section>;
}
