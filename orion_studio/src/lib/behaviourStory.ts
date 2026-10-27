import type { BehaviourEvent } from "../components/VoiceHistory";

/** What a row in the behaviour story is about. */
export type StoryKind = "did" | "said" | "skipped" | "noticed" | "sleep";
export type StoryFilter = "all" | StoryKind;
export interface StoryRow {
  id: string; at: string; kind: StoryKind; title: string; detail?: string; note?: string; remark?: boolean; seen?: string;
  /** Repeats of the same "didn't" in a row fold into one row from this time. */
  from?: string; count?: number; reasons?: { text: string; count: number }[];
}
export interface StoryBlock { id: string; visit: boolean; start: string; end: string; rows: StoryRow[]; idle: number }
export interface StoryCounts { visits: number; did: number; said: number; answered: number; skipped: number }

/** A visit ends when nobody was seen for this long, as on Orion. */
const VISIT_GAP_MS = 10 * 60 * 1000;

const remarkTopic: Record<string, string> = {
  person_arrived: "someone arriving", person_looking: "your look", new_object: "something new",
  hand_raised: "a raised hand", person_left: "someone leaving",
  light_care: "the room getting dark", break_nudge: "taking a break", goodnight: "saying goodnight",
};
const article = (thing: string) => (/^[aeiou]/i.test(thing) ? "an " : "a ") + thing;
const thing = (event: BehaviourEvent) => event.targetCategory && event.targetCategory !== "person" ? article(event.targetCategory) : "something";

function didTitle(event: BehaviourEvent): string {
  switch (event.behaviour) {
    case "mutual_gaze": return "Met your look";
    case "arrival_notice": return "Welcomed someone";
    case "farewell_watch": return "Watched someone leave";
    case "object_peek": return `Peeked at ${thing(event)}`;
    case "pet_notice": return `Noticed ${thing(event)}`;
    case "object_glance": return `Glanced at ${thing(event)}`;
    case "wake_stretch": return "Stretched after waking";
    case "sleep_settle": return "Settled down before sleeping";
    case "posture_home": return "Went back to its rest position";
    case "wave_back": return "Waved back";
    case "light_care": return "Looked up as the room got dark";
    case "break_nudge": return "Stretched after your long time at the desk";
    default: return event.behaviour.replaceAll("_", " ").replace(/^./, letter => letter.toUpperCase());
  }
}
function skippedTitle(event: BehaviourEvent): string {
  switch (event.behaviour) {
    case "remark": return `Stayed quiet about ${remarkTopic[event.targetCategory ?? ""] ?? "what it noticed"}`;
    case "mutual_gaze": return "Didn’t answer a look";
    case "arrival_notice": return "Didn’t welcome someone";
    case "farewell_watch": return "Didn’t watch someone leave";
    case "object_peek": return `Didn’t peek at ${thing(event)}`;
    case "wave_back": return "Didn’t wave back";
    default: return `Didn’t ${event.behaviour.replaceAll("_", " ")}`;
  }
}
/** Why Orion did not act, in the owner's words. */
export const skipReasons: Record<string, string> = {
  busy: "It was busy talking, playing music or running a routine.",
  reaction_running: "It was already reacting to something else.",
  spacing: "It did something a short while ago.",
  window: "It had already reacted several times in the last 10 minutes.",
  habituation: "It looked at this a short while ago.",
  focus: "You looked busy, so it left you alone.",
  once_per_visit: "It had already talked with you this visit.",
  night: "It stays quiet during quiet hours.",
  after_conversation: "You talked with it a few minutes ago.",
  ignored: "Its last remarks got no reply, so it waits longer.",
  cooldown: "It talked about this kind of thing recently.",
  hourly_limit: "It had already talked several times this hour.",
  ordinary_object: "It’s an everyday thing on a desk.",
  seen_today: "It already talked about this today.",
  asked_quiet: "You asked it to be quiet for a while.",
  pending: "It was about to say something else.",
  demo_only: "This happens only in Demo.",
  switched_off: "It is turned off in Settings.",
};
const restTitles: Record<string, string> = {
  "sleep:started": "Started going to sleep", "sleep:completed": "Fell asleep", "sleep:failed": "Couldn’t go to sleep",
  "sleep:interrupted": "Stopped going to sleep", "wake:started": "Started waking up", "wake:completed": "Woke up",
  "wake:failed": "Couldn’t wake up", "wake:interrupted": "Stopped waking up", "rest_transition:failed": "Couldn’t finish moving",
  "rest_policy:disabled": "Automatic sleep turned off",
};

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;
/** The situation saved with a decision, in plain words. */
export function whatOrionSaw(context: Record<string, unknown> | null | undefined): string | undefined {
  if (!context) return undefined;
  const number = (key: string) => typeof context[key] === "number" ? context[key] as number : undefined;
  const people = number("people") ?? 0;
  const parts = [people ? `${plural(people, "person", "people")} in view` : "Nobody in view"];
  if (context.looking) parts.push("someone looking at Orion");
  if (context.focused) parts.push("someone busy working");
  const minutes = number("visit_minutes");
  if (minutes != null) parts.push(`visit ${plural(minutes, "minute")} long`);
  if (context.busy) parts.push("Orion busy talking, playing or running a routine");
  if (typeof context.light === "string") parts.push(context.light === "off" ? "Orion’s light off" : `Orion’s light ${context.light.replaceAll("_", " ")}`);
  const lux = number("room_lux");
  if (lux != null) parts.push(lux < 15 ? "room dark" : lux < 100 ? "room dim" : "room bright");
  if (typeof context.liveliness === "string") parts.push(`${context.liveliness[0].toUpperCase()}${context.liveliness.slice(1)}`);
  const budget = number("budget");
  if (budget != null) parts.push(`${plural(budget, "gesture")} in the last 10 minutes`);
  return parts.join(" · ");
}

/** One readable row, or null for events that only matter in the technical log. */
export function storyRow(event: BehaviourEvent): StoryRow | null {
  const row = storyRowTitle(event);
  const seen = row && whatOrionSaw(event.context);
  return row && seen ? { ...row, seen } : row;
}
function storyRowTitle(event: BehaviourEvent): StoryRow | null {
  const base = { id: event.id, at: event.at };
  if (event.category === "rest") {
    const title = restTitles[`${event.behaviour}:${event.phase}`];
    return title ? { ...base, kind: "sleep", title } : null;
  }
  if (event.category !== "attention") return null;
  if (event.phase === "skipped") return { ...base, kind: "skipped", title: skippedTitle(event), detail: skipReasons[event.reason ?? ""] ?? "It couldn’t react right then." };
  switch (event.behaviour) {
    case "visit": return null;
    case "remark_words": return event.phase === "silent" ? { ...base, kind: "said", title: "Thought of something to say, then stayed quiet" }
      : { ...base, kind: "said", title: "Said", detail: event.reason ? `“${event.reason}”` : undefined };
    case "remark_spoken": return { ...base, kind: "said", remark: true, title: `Spoke up about ${remarkTopic[event.targetCategory ?? ""] ?? "what it noticed"}` };
    case "remark_answered": return { ...base, kind: "said", title: "You answered" };
    case "new_object": return { ...base, kind: "noticed", title: `Noticed something new: ${event.targetCategory ?? "an object"}` };
    case "lamp_state": {
      const brightness = event.target?.startsWith("brightness:") ? ` at ${event.target.slice(11)}%` : "";
      return { ...base, kind: "noticed", title: event.targetCategory === "off" ? "Light turned off" : `Light set to ${(event.targetCategory ?? "custom").replaceAll("_", " ")}${brightness}` };
    }
  }
  if (event.phase === "started" && !["gaze", "gaze_hold"].includes(event.behaviour)) return { ...base, kind: "did", title: didTitle(event) };
  return null;
}

const person = (event: BehaviourEvent) => event.targetCategory === "person" || event.behaviour === "person_presence";
const ms = (at: string) => new Date(at).getTime();

/**
 * When each event happened during a visit. Orion logs the start and end of
 * each visit; older logs without them fall back to person events and the
 * same 10 minute gap Orion uses.
 */
function visitFlags(sorted: BehaviourEvent[]): boolean[] {
  const marks = sorted.filter(event => event.behaviour === "visit");
  if (marks.length) {
    let inside = marks[0].phase === "ended";
    return sorted.map(event => {
      if (event.behaviour === "visit") inside = event.phase === "started";
      return inside;
    });
  }
  let lastPerson = -Infinity;
  return sorted.map(event => {
    const at = ms(event.at);
    if (person(event)) lastPerson = at;
    return at - lastPerson <= VISIT_GAP_MS;
  });
}

/** One more of the same "didn't": count it and its reason on the earlier row. */
function foldSkip(row: StoryRow, next: StoryRow) {
  row.reasons ??= [{ text: row.detail ?? "", count: 1 }];
  row.from ??= row.at;
  row.count = (row.count ?? 1) + 1;
  row.at = next.at;
  row.seen = next.seen ?? row.seen;
  const reason = row.reasons.find(item => item.text === next.detail);
  if (reason) reason.count++;
  else row.reasons.push({ text: next.detail ?? "", count: 1 });
  row.reasons.sort((a, b) => b.count - a.count);
  row.detail = row.reasons.length === 1 ? row.reasons[0].text : undefined;
}

/**
 * Groups raw events into visits and the time between them. Idle movements
 * collapse to a count; a movement that stopped early notes it on its row.
 */
export function behaviourStory(events: BehaviourEvent[], filter: StoryFilter = "all"): StoryBlock[] {
  const sorted = [...events].sort((a, b) => ms(a.at) - ms(b.at) || a.id.localeCompare(b.id));
  const outcome = new Map<string, string>();
  for (const event of sorted) {
    if (event.actionId && event.category === "attention" && (event.phase === "interrupted" || event.phase === "failed"))
      outcome.set(event.actionId, event.phase === "failed" ? "Didn’t finish" : "Stopped early");
  }
  const flags = visitFlags(sorted);
  const blocks: StoryBlock[] = [];
  sorted.forEach((event, index) => {
    const inVisit = flags[index], startsVisit = event.behaviour === "visit" && event.phase === "started";
    let block = blocks.at(-1);
    if (block?.visit && event.behaviour === "visit" && event.phase === "ended") block.end = event.at;
    if (!block || block.visit !== inVisit || startsVisit) {
      block = { id: event.id, visit: inVisit, start: event.at, end: event.at, rows: [], idle: 0 };
      blocks.push(block);
    }
    block.end = event.at;
    if (event.category === "idle" && event.phase === "started") { if (filter === "all" || filter === "did") block.idle++; return; }
    const row = storyRow(event);
    if (!row || (filter !== "all" && row.kind !== filter)) return;
    // The words follow the remark they belong to: show them on one row.
    const spoken = event.behaviour === "remark_words" ? [...block.rows].reverse().find(item => item.remark && !item.detail && ms(event.at) - ms(item.at) <= 120_000) : undefined;
    if (spoken) {
      if (event.phase === "silent") spoken.title = "Thought of something to say, then stayed quiet";
      else spoken.detail = row.detail;
      return;
    }
    const last = block.rows.at(-1);
    if (row.kind === "skipped" && last?.kind === "skipped" && last.title === row.title) {
      foldSkip(last, row);
      return;
    }
    const note = event.actionId ? outcome.get(event.actionId) : undefined;
    block.rows.push(note && row.kind === "did" ? { ...row, note } : row);
  });
  // Newest first, in the list of visits and inside each visit.
  return blocks.filter(block => block.rows.length || block.idle || (block.visit && filter === "all"))
    .reverse().map(block => ({ ...block, rows: block.rows.reverse() }));
}

export function storyCounts(events: BehaviourEvent[]): StoryCounts {
  const rows = events.map(storyRow).filter((row): row is StoryRow => !!row);
  return {
    visits: behaviourStory(events).filter(block => block.visit).length,
    did: rows.filter(row => row.kind === "did").length,
    said: events.filter(event => event.behaviour === "remark_words" && event.phase === "observed").length,
    answered: events.filter(event => event.behaviour === "remark_answered").length,
    skipped: rows.filter(row => row.kind === "skipped").length,
  };
}
