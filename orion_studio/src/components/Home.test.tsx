import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { Home } from "./Home";
import type { GatewayStatus, ProjectCatalog } from "../types";

vi.mock("./RobotViewport", () => ({ RobotViewport: () => null }));
const catalog = { poses: { attentive: { positions: {} } } } as unknown as ProjectCatalog;
const props = { catalog, theme: "dark" as const, voiceLabel: "Off", connection: null, status: null,
  onConnect: vi.fn(), onVoice: vi.fn(), onCreate: vi.fn(), onRefresh: async () => {}, onNotice: vi.fn(), onRun: vi.fn() };

describe("Home control availability", () => {
  it("disables hardware actions while disconnected but keeps lamp drafts and voice access available", () => {
    const html = renderToStaticMarkup(<Home {...props} />);
    expect(html).toMatch(/role="switch"[^>]*disabled=""/);
    expect(html).toMatch(/<button class="oh-apply" disabled=""/);
    expect(html).toMatch(/<button class="oh-talk"[^>]*>/);
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
