import { expect, it } from "vitest";
import { behaviourStory, storyCounts, storyRow } from "./behaviourStory";
import type { BehaviourEvent } from "../components/VoiceHistory";

let next = 0;
const event = (at: string, behaviour: string, phase: string, patch: Partial<BehaviourEvent> = {}): BehaviourEvent => ({
  id: `e${next++}`, at: `2026-10-26T${at}:00Z`, category: "attention", behaviour, phase,
  target: null, targetCategory: null, reason: null, durationSeconds: null, actionId: null, ...patch,
});

it("tells a visit as what Orion did, said and chose not to do, with plain reasons", () => {
  const events = [
    event("08:00", "idle_breathe", "started", { category: "idle" }),
    event("09:00", "visit", "started", { targetCategory: "person" }),
    event("09:00", "arrival_notice", "started", { targetCategory: "person", actionId: "a" }),
    event("09:01", "arrival_notice", "interrupted", { targetCategory: "person", actionId: "a" }),
    event("09:01", "remark_spoken", "observed", { targetCategory: "person_arrived" }),
    event("09:01", "remark_words", "observed", { reason: "Morning! Coffee first?" }),
    event("09:02", "remark_answered", "observed"),
    event("09:05", "idle_glance", "started", { category: "idle" }),
    event("09:06", "idle_glance", "started", { category: "idle" }),
    event("09:30", "remark", "skipped", { targetCategory: "person_looking", reason: "once_per_visit" }),
    event("09:40", "object_peek", "skipped", { targetCategory: "cup", reason: "busy" }),
    event("10:00", "visit", "ended", { targetCategory: "person" }),
    event("10:20", "sleep", "completed", { category: "rest" }),
    event("10:21", "log_gap", "observed", { category: "system" }),
  ];
  const [quiet, visit, before] = behaviourStory(events);
  expect(before).toMatchObject({ visit: false, idle: 1, rows: [] });
  expect(visit.visit).toBe(true);
  expect(visit.end).toBe("2026-10-26T10:00:00Z");
  expect(visit.idle).toBe(2);
  expect(visit.rows.map(row => row.title)).toEqual(["Welcomed someone", "Spoke up about someone arriving", "You answered",
    "Stayed quiet about your look", "Didn’t peek at a cup"]);
  expect(visit.rows[0].note).toBe("Stopped early");
  expect(visit.rows[1].detail).toBe("“Morning! Coffee first?”");
  expect(visit.rows[3].detail).toContain("already talked with you this visit");
  expect(quiet.rows.map(row => row.title)).toEqual(["Fell asleep"]);
  expect(behaviourStory(events, "skipped").flatMap(block => block.rows).length).toBe(2);
  expect(storyCounts(events)).toEqual({ visits: 1, did: 1, said: 1, answered: 1, skipped: 2 });
});

it("finds visits from person events in logs written before Orion logged visits", () => {
  const events = [
    event("09:00", "person_presence", "observed", { targetCategory: "person" }),
    event("09:05", "mutual_gaze", "started", { targetCategory: "person" }),
    event("09:30", "object_peek", "started", { targetCategory: "book" }),
  ];
  const blocks = behaviourStory(events);
  expect(blocks.map(block => [block.visit, block.rows.map(row => row.title)])).toEqual([
    [false, ["Peeked at a book"]], [true, ["Met your look"]],
  ]);
});

it("names the lively behaviours in plain words", () => {
  const rows = [
    event("21:00", "wave_back", "started", { targetCategory: "person" }),
    event("21:10", "break_nudge", "started", { targetCategory: "person" }),
    event("21:11", "remark_spoken", "observed", { targetCategory: "break_nudge" }),
    event("21:30", "remark_spoken", "observed", { targetCategory: "goodnight" }),
  ].map(storyRow).map(row => row?.title);
  expect(rows).toEqual(["Waved back", "Stretched after your long time at the desk", "Spoke up about taking a break", "Spoke up about saying goodnight"]);
});

it("says what Orion saw when it decided", () => {
  const row = storyRow(event("21:00", "remark", "skipped", { reason: "focus", targetCategory: "person_looking",
    context: { people: 1, looking: false, focused: true, visit_minutes: 42, busy: false, light: "warm_white 60%", room_lux: 8, liveliness: "lively", budget: 1 } }));
  expect(row?.seen).toBe("1 person in view · someone busy working · visit 42 minutes long · Orion’s light warm white 60% · room dark · Lively · 1 gesture in the last 10 minutes");
  expect(storyRow(event("21:00", "wave_back", "started"))?.seen).toBeUndefined();
});
