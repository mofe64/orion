import { isTauri, invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import "./VoiceHistory.css";
import { failure, forOwner, type Failure } from "../lib/feedback";
import { FailureNotice } from "./FailureNotice";
import { behaviourStory, storyCounts, type StoryFilter } from "../lib/behaviourStory";

interface Summary {
  conversationId: string; startedAt: string; updatedAt: string; expiresAt: string;
  preview: string; turnCount: number; retired: boolean; extraction: string;
}
interface Page<T> { items: T[]; nextCursor: string | null }
interface Entry { at: string; event: Record<string, unknown> }
export interface HistoryTurn {
  sessionId: string; startedAt: string; updatedAt: string; command: string;
  reply: string | null; error: string | null; delivery: string; events: Entry[];
}
interface Conversation {
  conversation: { conversationId: string; startedAt: string; expiresAt: string; retiredAt: string | null; extraction: string };
  turns: HistoryTurn[];
}
export interface BehaviourEvent {
  id: string; at: string; category: string; behaviour: string; phase: string;
  target: string | null; targetCategory: string | null; reason: string | null;
  durationSeconds: number | null; actionId: string | null;
  /** What Orion saw when it decided, for decisions logged since 0.1.16. */
  context?: Record<string, unknown> | null;
}
export function loadHistory(conversationId: string | null, before: string | null) {
  const message = conversationId ? "Studio couldn't open this conversation. Please try again." : before ? "Studio couldn't load older conversations. Please try again." : "Studio couldn't load Orion's conversation history. Please try again.";
  return forOwner(() => invoke<Page<Summary> | Conversation>("load_voice_history", { conversationId, before }), message);
}
export function loadBehaviours(before: string | null, category: string | null) {
  return forOwner(() => invoke<Page<BehaviourEvent>>("load_behaviour_history", { before, category }), "Studio couldn't load Orion's behaviour history. Please try again.");
}
const time = (date: string) => new Date(date).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit" });
const day = (date: string) => new Date(date).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long", year: "numeric" });
const ms = (value: unknown) => typeof value === "number" && Number.isFinite(value) ? value >= 1000 ? `${(value / 1000).toFixed(2)} s` : `${Math.round(value)} ms` : "—";
const str = (value: unknown) => typeof value === "string" ? value : "";
const readable = (value: unknown) => JSON.stringify(value, null, 2) ?? "—";
const toolName = (name: string) => ({ set_timer: "Set timer", set_alarm: "Set alarm", list_alerts: "Check alarms and timers", cancel_alert: "Cancel alert", stop_alert: "Stop alert", set_lighting: "Change light", set_mode: "Change mode", go_to_sleep: "Go to sleep", search_memories: "Search memories", append_memory: "Save memory" } as Record<string,string>)[name] ?? name.replaceAll("_", " ");
function title(event: Record<string, unknown>): string | null {
  switch (event.type) {
    case "agent.started": return "Sent to the agent";
    case "agent.progress": return str(event.message) || "Agent is working";
    case "agent.tool": return toolName(str(event.name));
    case "synthesis.started": return "Creating Orion’s voice";
    case "speech.started": return "Playback started";
    case "speech.completed": return "Playback finished";
    case "session.interrupted": return "Playback interrupted";
    case "agent.failed": case "worker.error": return "Something went wrong";
    case "stage.timing": return str(event.stage).replaceAll("_", " ");
    default: return null;
  }
}
export function ConversationTimeline({ turns }: { turns: HistoryTurn[] }) {
  return <ol className="conversation-exchanges">{turns.map(turn => <li key={turn.sessionId}>
    <time dateTime={turn.startedAt}>{time(turn.startedAt)}</time>
    <div className="conversation-message"><strong>You</strong><p>{turn.command}</p></div>
    {turn.reply && <div className="conversation-message orion-message"><strong>Orion</strong><p>{turn.reply}</p></div>}
    {turn.delivery === "interrupted" && <p className="settings-help">Reply interrupted; it may not have been heard in full.</p>}
    {turn.delivery === "pending" && <p role="status" className="settings-help">Request in progress</p>}
    {turn.error && <p className="settings-error">{turn.error}</p>}
    <details className="conversation-processing"><summary>Actions and processing details</summary><ol className="voice-history-timeline">{turn.events.map((entry,index) => {
      const label = title(entry.event); if (!label) return null;
      const event = entry.event;
      return <li key={index}><div className="voice-history-step"><time dateTime={entry.at}>{time(entry.at)}</time><h3>{label}</h3>
        {event.type === "worker.error" && <p>{str(event.message)}</p>}
        {event.type === "agent.tool" && <><p>{event.success ? "Completed" : "Failed"} · {ms(event.durationMs)}</p><details><summary>Request and result</summary><pre>{readable({ request:event.arguments, result:event.result })}</pre></details></>}
        {event.type === "stage.timing" && <p>{ms(event.durationMs)}</p>}
      </div></li>;
    })}</ol></details>
  </li>)}</ol>;
}
const behaviourNames: Record<string,string> = {
  new_object: "Noticed an object newly present in the recent scene", person_presence: "Noticed a person",
  object_glance: "Object glance", object_peek: "Object peek", mutual_gaze: "Mutual gaze", gaze: "Gaze",
  gaze_hold: "Held gaze", rest_state: "Rest state", arrival_notice: "Arrival notice", pet_notice: "Pet notice", sleep: "Sleep", wake: "Wake",
  rest_transition: "Rest transition", rest_policy: "Rest policy", log_gap: "Behaviour log gap",
};
export function BehaviourTimeline({ events }: { events: BehaviourEvent[] }) {
  return <ol className="voice-history-timeline behaviour-timeline">{events.map(event => <li key={event.id}><div className="voice-history-step">
    <time dateTime={event.at}>{day(event.at)} · {time(event.at)}</time>
    <h3>{behaviourNames[event.behaviour] ?? event.behaviour.replaceAll("_", " ")}</h3>
    <p>{event.phase}{event.targetCategory ? ` · ${event.targetCategory}` : ""}{event.durationSeconds != null ? ` · ${event.durationSeconds.toFixed(1)} s` : ""}</p>
    {event.reason && <p>{event.reason}</p>}
  </div></li>)}</ol>;
}

const clock = (date: string) => new Date(date).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
export const storyFilters: [StoryFilter | "technical", string][] = [
  ["all", "Everything"], ["did", "What it did"], ["said", "What it said"], ["skipped", "What it chose not to do"],
  ["noticed", "What it noticed"], ["sleep", "Sleep and wake"], ["technical", "Technical log"],
];
/** Behaviour history as visits: what Orion did, said and chose not to do, and why. */
export function BehaviourStory({ events, filter }: { events: BehaviourEvent[]; filter: StoryFilter }) {
  const blocks = behaviourStory(events, filter);
  const counts = storyCounts(events);
  let previousDay = "";
  return <div className="behaviour-story">
    <p className="behaviour-summary">{counts.visits} {counts.visits === 1 ? "visit" : "visits"} · {counts.did} {counts.did === 1 ? "thing" : "things"} done · {counts.said} said ({counts.answered} answered) · {counts.skipped} chosen not to do</p>
    {!blocks.length && <p className="voice-history-empty">Nothing to show for this choice yet.</p>}
    {blocks.map(block => { const heading = day(block.start), showDay = heading !== previousDay; previousDay = heading;
      return <section key={block.id} className={`behaviour-block${block.visit ? " visit" : ""}`}>{showDay && <h2>{heading}</h2>}
        <header><strong>{block.visit ? "Someone was here" : "Nobody around"}</strong><time dateTime={block.start}>{clock(block.start)} – {clock(block.end)}</time></header>
        <ol>{block.rows.map(row => <li key={row.id} className={`behaviour-row ${row.kind}`}><time dateTime={row.at}>{clock(row.at)}</time>
          <div><strong>{row.title}</strong>{row.detail && <p>{row.detail}</p>}{row.note && <p className="settings-help">{row.note}</p>}
            {row.seen && <details className="behaviour-seen"><summary>What Orion saw</summary><p>{row.seen}</p></details>}</div></li>)}
          {block.idle > 0 && <li className="behaviour-row idle"><span /><div><p>{block.idle} small idle {block.idle === 1 ? "movement" : "movements"}</p></div></li>}
        </ol></section>; })}
  </div>;
}

export function VoiceHistory({ connected, desktop = isTauri(), connectionScope = connected }: { connected: boolean; desktop?: boolean; connectionScope?: unknown }) {
  const available = connected && desktop;
  const [view, setView] = useState<"conversations" | "behaviours">("conversations");
  const [filter, setFilter] = useState<StoryFilter | "technical">("all");
  const [items, setItems] = useState<Summary[]>([]);
  const [behaviours, setBehaviours] = useState<BehaviourEvent[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [conversation, setConversation] = useState<Conversation | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Failure | null>(null);
  const epoch = useRef(0), detailRequest = useRef(0);
  const open = async (id: string, generation = epoch.current) => {
    const request = ++detailRequest.current; setSelected(id); setConversation(null);
    try { const result = await loadHistory(id,null) as Conversation;
      if (generation === epoch.current && request === detailRequest.current) setConversation(result);
    } catch (reason) { if (generation === epoch.current && request === detailRequest.current) setError(failure("Studio couldn't open this conversation. Please try again.",reason)); }
  };
  const refresh = async (older = false) => {
    const generation = epoch.current; setLoading(true); setError(null);
    try {
      if (view === "behaviours") {
        const page = await loadBehaviours(older ? nextCursor : null, null);
        if (generation !== epoch.current) return;
        setBehaviours(previous => older ? [...previous,...page.items] : page.items); setNextCursor(page.nextCursor);
      } else {
        const page = await loadHistory(null,older ? nextCursor : null) as Page<Summary>;
        if (generation !== epoch.current) return;
        setItems(previous => older ? [...previous,...page.items] : page.items); setNextCursor(page.nextCursor);
        if (!older) {
          const id = selected && page.items.some(item => item.conversationId === selected) ? selected : page.items[0]?.conversationId;
          if (id) await open(id,generation); else { setSelected(null); setConversation(null); }
        }
      }
    } catch (reason) { if (generation === epoch.current) setError(failure("Studio couldn't load Orion's history. Please try again.",reason)); }
    finally { if (generation === epoch.current) setLoading(false); }
  };
  useEffect(() => {
    epoch.current++; detailRequest.current++; setError(null); setNextCursor(null);
    setItems([]); setBehaviours([]); setConversation(null); setSelected(null);
    if (available) void refresh(); else setLoading(false);
    return () => { epoch.current++; detailRequest.current++; };
  },[available,view,connectionScope]);
  let previousDay = "";
  return <section className="settings-page voice-history" aria-labelledby="voice-history-title">
    <header className="voice-history-heading"><div><h1 id="voice-history-title">History</h1><p>{view === "behaviours" ? "What Orion did, said and chose not to do, visit by visit." : "Conversations last 24 hours from the first turn."}</p></div><button className="quiet-button" disabled={!available || loading} onClick={() => void refresh()}>{loading ? "Loading…" : "Refresh"}</button></header>
    <div className="history-controls"><div role="group" aria-label="History view">{(["conversations","behaviours"] as const).map(value => <button className="quiet-button" key={value} aria-pressed={view === value} onClick={() => setView(value)}>{value === "conversations" ? "Conversations" : "Behaviours"}</button>)}</div>
      {view === "behaviours" && <label>Show <select value={filter} onChange={event => setFilter(event.target.value as StoryFilter | "technical")}>{storyFilters.map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>}
    </div>
    {!desktop ? <p className="settings-help">Open Orion Studio’s desktop app and connect Orion to read history.</p> : !connected && <p className="settings-help">Connect Orion to read its history.</p>}
    <FailureNotice value={error}/>
    {view === "behaviours" ? <div className="voice-history-layout behaviour-history">{filter === "technical" ? <BehaviourTimeline events={behaviours}/> : behaviours.length > 0 && <BehaviourStory events={behaviours} filter={filter}/>}{!behaviours.length && <p className="voice-history-empty">{loading ? "Reading Orion’s behaviours…" : available ? "No behaviour events recorded yet." : "Connect in the desktop app to read saved behaviours."}</p>}{nextCursor && <button className="quiet-button" disabled={loading} onClick={() => void refresh(true)}>Load older behaviours</button>}</div> :
      <div className="voice-history-layout"><aside className="voice-history-list" aria-label="Conversations">
        {!items.length && <p className="voice-history-empty">{loading ? "Reading Orion’s history…" : available ? "No saved conversations yet." : "Connect in the desktop app to read saved conversations."}</p>}
        {items.map(item => {const heading=day(item.startedAt), showDay=heading!==previousDay;previousDay=heading;return <div key={item.conversationId}>{showDay && <h2>{heading}</h2>}<button className={`voice-history-item${item.conversationId === selected ? " selected" : ""}`} aria-current={item.conversationId === selected ? "true" : undefined} onClick={() => void open(item.conversationId)}><time dateTime={item.startedAt}>{time(item.startedAt)} · {item.turnCount} {item.turnCount===1 ? "turn" : "turns"}</time><strong>{item.preview}</strong><span>{item.retired ? "Archived" : "Active"} · {time(item.updatedAt)}</span></button></div>;})}
        {nextCursor && <button className="quiet-button voice-history-older" disabled={loading} onClick={() => void refresh(true)}>Load older conversations</button>}
      </aside><article className="voice-history-detail">{conversation ? <><header><p className="eyebrow">{day(conversation.conversation.startedAt)}</p><h2>{conversation.turns[0]?.command}</h2><p>{time(conversation.conversation.startedAt)} – {day(conversation.conversation.expiresAt)} · {time(conversation.conversation.expiresAt)}</p>{conversation.conversation.retiredAt && <p>Memory extraction: {conversation.conversation.extraction}</p>}</header><ConversationTimeline turns={conversation.turns}/></> : <p className="voice-history-empty">{selected ? error ? "Use Refresh to try opening this conversation again." : "Opening conversation…" : "Choose a conversation to read its exchanges."}</p>}</article></div>}
  </section>;
}
