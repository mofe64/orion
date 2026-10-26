import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { CharacterPanel, RecorderChart } from "./Recorder";
import { median, type Initiative, type RecorderRow } from "../lib/recorder";

const row = (t: number, temp: number): RecorderRow => ({ t, reachable: true, joints: { base_yaw_joint: { temp, current: 300, volt_min: 6, status: 0 }, head_roll_joint: { temp: temp + 5, current: 100, volt_min: 5.9, status: 0 } },
  loop: { work_ms: 3, delay_ms: 1, late: 0 }, bus_errors: 0, cpu_temp: 55, cpu: 20, ram: 40 });

it("draws one line for each servo with a legend and readable axis", () => {
  const html = renderToStaticMarkup(<RecorderChart title="Servo temperature, highest" unit="°C" hours={1} rows={[row(1000, 40), row(1010, 44)]}
    series={[{ key: "base", label: "Base", color: "#3987e5", value: item => item.joints.base_yaw_joint.temp }, { key: "neck", label: "Neck turn", color: "#d95926", value: item => item.joints.head_roll_joint.temp }]} />);
  expect(html.match(/<path /g)).toHaveLength(2);
  for (const text of ["Base", "Neck turn", "°C", "Servo temperature"]) expect(html).toContain(text);
  expect(renderToStaticMarkup(<RecorderChart title="Empty" unit="V" hours={1} rows={[]} series={[]} />)).toContain("No readings yet");
});

it("explains what Orion sees and why it would stay quiet", () => {
  const initiative: Initiative = {
    situation: { people: 1, looking: false, focused: true, visit: 100, away_since: null, hour: 21.5, busy: false, lamp: { on: false, brightness: 0, mood: "warm_white" }, room_lux: 6, liveliness: "lively", at: 1900 },
    curiosity: { gestures_used: 1, gestures_allowed: 3, next_gesture_seconds: 30, focused: true, curiosity: "nosy", remembered_targets: 2 },
    quirks: { curiosity: "nosy", chattiness: "reserved" },
    lively: { desk_seconds: 1800, dark_seconds: 40, goodnight_rest_seconds: null },
    remark_refusals: { person_looking: "focus", light_care: null, goodnight: "pending" },
  };
  const html = renderToStaticMarkup(<CharacterPanel initiative={initiative} ignored={1} />);
  for (const text of ["Lively", "nosy curiosity", "busy working", "30 min", "Off", "6 lux", "1 of 3", "Answer a look", "You looked busy", "Offer light", "Would speak", "Ignored remarks"]) expect(html).toContain(text);
  expect(renderToStaticMarkup(<CharacterPanel />)).toContain("Update Orion");
});

it("finds the usual value", () => {
  expect(median([3, 1, 2])).toBe(2);
  expect(median([4, 1, 2, 3])).toBe(2.5);
  expect(median([])).toBeNull();
});
