import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { Home } from "./Home";
import type { GatewayStatus, ProjectCatalog } from "../types";

vi.mock("./RobotViewport", () => ({ RobotViewport: () => null }));
const catalog = { poses: { attentive: { positions: {} } } } as unknown as ProjectCatalog;
const props = { catalog, theme: "dark" as const, voiceLabel: "Off", connection: null, status: null,
  onConnect: vi.fn(), onVoice: vi.fn(), onCreate: vi.fn(), onRefresh: async () => {}, onNotice: vi.fn(), onRun: vi.fn() };

describe("Home control availability", () => {
  it("shows runtime rest and light power instead of a stale lamp command", () => {
    const status = { character: { enabled: false }, runtime: { motion: null }, scene: { active: null }, speech: { active: null },
      rest: { state: "resting", light_on: false, error: null } } as unknown as GatewayStatus;
    const html = renderToStaticMarkup(<Home {...props} status={status} connection={{ url: "http://localhost:7447", token: "test" }} />);
    expect(html).toContain("Resting · torque off");
    expect(html).toContain("Off while resting · lamp setting saved");
    expect(html).toMatch(/aria-label="Lamp power"[^>]*aria-checked="false"[^>]*disabled=""/);
    expect(html).not.toContain("Character off");
  });
  it("reports a rest failure without claiming torque was released", () => {
    const status = { character: { enabled: false }, runtime: { motion: null }, scene: { active: null }, speech: { active: null },
      rest: { state: "fault", light_on: false, error: "Rest did not complete; torque has not been released." } } as unknown as GatewayStatus;
    const html = renderToStaticMarkup(<Home {...props} status={status} connection={{ url: "http://localhost:7447", token: "test" }} />);
    expect(html).toContain("Rest / wake needs attention");
    expect(html).toContain("torque has not been released");
    expect(html).not.toContain("Resting · torque off");
  });
  it("disables hardware actions while disconnected but keeps lamp drafts available", () => {
    const html = renderToStaticMarkup(<Home {...props} />);
    expect(html).toMatch(/role="switch"[^>]*disabled=""/);
    expect(html).toMatch(/<button class="oh-apply" disabled=""/);
    expect(html).toMatch(/aria-label="Orion listening"[^>]*aria-checked="false"[^>]*disabled=""/);
    expect(html).not.toContain('type="color"');
    expect(html).not.toContain('id="lamp-color"');
    expect(html).toContain("Custom color");
    expect(html).not.toContain("Turn off light");
    expect(html).toContain("Model preview, not live position");
  });
  it("does not invent a lamp state after connecting and blocks expressions during foreground work", () => {
    const status = { character: { enabled: false }, runtime: { motion: null }, scene: { active: { run_id: 2 } }, speech: { active: null } } as unknown as GatewayStatus;
    const html = renderToStaticMarkup(<Home {...props} status={status} connection={{ url: "http://localhost:7447", token: "test" }} />);
    expect(html).toContain("Not set in this session");
    const expressions = html.split('class="oh-expression-list"')[1];
    expect(expressions.match(/disabled=""/g)).toHaveLength(2);
    expect(html).not.toMatch(/<button class="oh-apply" disabled/);
  });
});
