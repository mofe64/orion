import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { Home, homeFeedbackArea } from "./Home";
import type { GatewayStatus, ProjectCatalog } from "../types";

vi.mock("./RobotViewport", () => ({ RobotViewport: () => null }));
const catalog = { poses: { attentive: { positions: {} } }, scenes: { acknowledge_left: { name: "acknowledge_left", source: "built_in" }, acknowledge_right: { name: "acknowledge_right", source: "built_in" } } } as unknown as ProjectCatalog;
const props = { catalog, theme: "dark" as const, voiceLabel: "Off", connection: null, status: null,
  onConnect: vi.fn(), onVoice: vi.fn(), onCreate: vi.fn(), onRefresh: async () => {}, onRun: vi.fn() };

describe("Home control availability", () => {
  it("has no duplicate connected notice and keeps listening feedback beside its switch", () => {
    const html = renderToStaticMarkup(<Home {...props} connection={{ url: "http://test", token: "test" }} voiceNotice="Orion listening is off." />);
    expect(html).not.toContain("Connected to Orion.");
    expect(html).not.toContain("Connect Orion to use character");
    expect(html.indexOf("Orion listening is off.")).toBeGreaterThan(html.indexOf('aria-label="Orion listening"'));
    expect(html.indexOf("Orion listening is off.")).toBeLessThan(html.indexOf('aria-label="Lamp controls"'));
  });
  it.each([["Go to rest", "mode"], ["Lamp mode", "mode"], ["Pause mode", "mode"], ["Resume mode", "mode"], ["Warm white light", "lamp"], ["Custom color", "lamp"], ["Light off", "lamp"], ["Cancel alert", "alert"], ["Cancel alert series", "alert"], ["Acknowledge left", "expression"]])("keeps %s feedback in the %s control area", (action, area) => {
    expect(homeFeedbackArea(action)).toBe(area);
  });
  it("shows runtime rest and light power instead of a stale lamp command", () => {
    const status = { character: { enabled: false }, runtime: { motion: null }, scene: { active: null }, speech: { active: null },
      rest: { state: "resting", light_on: false, error: null } } as unknown as GatewayStatus;
    const html = renderToStaticMarkup(<Home {...props} status={status} connection={{ url: "http://localhost:7447", token: "test" }} />);
    expect(html).toContain("Resting · torque off");
    expect(html).toContain("Off while resting · lamp setting saved");
    expect(html).toMatch(/aria-label="Lamp power"[^>]*aria-checked="false"[^>]*disabled=""/);
    expect(html).not.toContain("Character off");
    expect(html).toMatch(/disabled=""[^>]*>.*?<strong>Go to rest/s);
  });
  it("reports a rest failure without claiming torque was released", () => {
    const status = { character: { enabled: false }, runtime: { motion: null }, scene: { active: null }, speech: { active: null },
      rest: { state: "fault", light_on: false, error: "Rest did not complete; torque has not been released." } } as unknown as GatewayStatus;
    const html = renderToStaticMarkup(<Home {...props} status={status} connection={{ url: "http://localhost:7447", token: "test" }} />);
    expect(html).toContain("Rest / wake needs attention");
    expect(html).toContain("torque has not been released");
    expect(html).not.toContain("Resting · torque off");
  });
  it("shows one connect action and a compact placeholder without dead controls", () => {
    const html = renderToStaticMarkup(<Home {...props} />);
    expect(html).toContain("Your Orion awaits.");
    expect(html.match(/class="oh-connect"/g)).toHaveLength(1);
    expect(html).toContain("Your scenes and previews are available in Animation.");
    expect(html).not.toContain('role="switch"');
    expect(html).toContain(">Tracking</strong>");
    expect(html).toContain("Connect Orion to change tracking");
    expect(html).toMatch(/fieldset[^>]*disabled=""/);
    expect(html).not.toContain('aria-label="Lamp controls"');
    expect(html).not.toContain('aria-label="Orion mode"');
    expect(html).not.toContain("Hello, Orion.");
  });
  it("does not invent a lamp state after connecting and blocks expressions during foreground work", () => {
    const status = { character: { enabled: false }, runtime: { motion: null }, scene: { active: { run_id: 2 } }, speech: { active: null } } as unknown as GatewayStatus;
    const html = renderToStaticMarkup(<Home {...props} status={status} connection={{ url: "http://localhost:7447", token: "test" }} />);
    expect(html).toContain("Not set in this session");
    const expressions = html.split('class="oh-expression-list"')[1].split("</div>")[0];
    expect(expressions.match(/disabled=""/g)).toHaveLength(2);
    expect(html).not.toMatch(/id="lamp-brightness"[^>]*disabled/);
    expect(html).not.toContain('class="oh-dial-value"');
    expect(html).not.toContain('class="oh-apply"');
    expect(html).toContain("Orion does not report its brightness");
    expect(html).toContain("Model preview, not live position");
  });
});

it.each([true, false])("keeps pause/resume on Home with enabled=%s and separates rest from mode choices", enabled => {
  const status = { character: { enabled, state: "home_idle" }, runtime: { motion: null }, scene: { active: null }, speech: { active: null } } as unknown as GatewayStatus;
  const html = renderToStaticMarkup(<Home {...props} status={status} connection={{ url: "http://test", token: "test" }} />);
  expect(html).toContain(enabled ? ">Pause mode</button>" : ">Resume mode</button>");
  expect(html).toContain(enabled ? "Idle mode</span>" : "Idle mode · paused</span>");
  const mode = html.slice(html.indexOf('aria-label="Orion mode"'), html.indexOf('class="oh-rest-action"'));
  expect(mode).toContain("Idle mode"); expect(mode).toContain("Lamp mode");
  expect(mode).not.toContain("Go to rest");
  expect(html).toContain("Go to rest");
  expect(html).not.toContain("Character · home idle");
});

it.each(["scene", "speech", "motion", "alarm"])("blocks owner Expressions during %s foreground activity", activity => {
  const status = { character:{enabled:false},runtime:{motion:activity === "motion" ? {name:"owner_motion"} : null},scene:{active:activity === "scene" ? {run_id:1}:null},speech:{active:activity === "speech" ? {run_id:2}:null},routines:activity === "alarm" ? {ringing:true,alerts:[]}:undefined } as unknown as GatewayStatus;
  const ownerCatalog = {...catalog,scenes:{...catalog.scenes,owner_wave:{format_version:2,name:"owner_wave",source:"user",description:"",motion:[],lighting:[],audio:[],finish:{anchor:"final_pose",lighting:"off"}}}} as ProjectCatalog;
  const html = renderToStaticMarkup(<Home {...props} catalog={ownerCatalog} connection={{url:"http://test",token:"test"}} status={status} />);
  const expressions=html.split('class="oh-expression-list"')[1].split("</div>")[0];
  expect(expressions).toContain("Owner wave");
  expect(expressions.match(/disabled=""/g)).toHaveLength(3);
});

it("shows reminder recurrence, a form, and recent missed reminders", () => {
  const status = {character:{enabled:false},runtime:{motion:null},scene:{active:null},speech:{active:null},routines:{ringing:false,alerts:[{id:1,kind:"reminder",label:"Stretch",state:"pending",due_unix:Date.now()/1000+60,repeat:{days:["mon","tue"],time:"09:00"}},{id:2,kind:"reminder",label:"Missed bins",state:"missed",due_unix:Date.now()/1000-3600},{id:3,kind:"reminder",label:"Old reminder",state:"missed",due_unix:Date.now()/1000-90000}]}} as unknown as GatewayStatus;
  const html = renderToStaticMarkup(<Home {...props} connection={{url:"http://test",token:"test"}} status={status} />);
  expect(html).toContain("mon, tue at 09:00"); expect(html).toContain("Missed bins"); expect(html).not.toContain("Old reminder");
  expect(html).toContain('aria-label="New reminder"');
});

it("offers distinct occurrence and series cancellation for repeating alerts", () => {
  const status = { character: {enabled: true}, runtime: {motion: null}, scene: {active: null}, speech: {active: null}, routines: { mode: "idle", ringing: true, alerts: [{ id: 1, kind: "alarm", label: "Gym", due_unix: 101, state: "ringing", repeat: { days: ["mon"], time: "07:00" } }] } } as unknown as GatewayStatus;
  const html = renderToStaticMarkup(<Home {...props} status={status} connection={{ url: "http://test", token: "test" }} />);
  expect(html).toContain(">Cancel occurrence</button>");
  expect(html).toContain('aria-label="Cancel series Gym"');
});
