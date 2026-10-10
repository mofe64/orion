import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import { compileMotionPreview, previewScene, publishScene } from "../lib/gateway";
import { failure } from "../lib/feedback";
import { EditorFeedback } from "./EditorFeedback";

afterEach(() => vi.unstubAllGlobals());

describe("editor feedback", () => {
  it.each([
    { action: "preview", message: "Studio couldn't prepare this preview. Please try again.", request: () => compileMotionPreview({ url: "http://test", token: "test" }, "look_left", "home") },
    { action: "publish", message: "Studio couldn't publish this to Orion. Please try again.", request: () => publishScene({ url: "http://test", token: "test" }, {}) },
    { action: "run", message: "Studio couldn't play this on Orion. Please try again.", request: () => previewScene({ url: "http://test", token: "test" }, {}) },
  ])("keeps $action gateway diagnostics inside collapsed details", async ({ message, request }) => {
    const detail = "orion-trajectory: Calibration hardware does not match --hardware, or is simulation-only.";
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({ error: { message: detail } }), { status: 400 })));
    let html = "";
    try { await request(); }
    catch (error) { html = renderToStaticMarkup(<EditorFeedback value={failure(message, error)} />); }
    expect(html).toContain(`<pre>${detail}</pre>`);
    expect(html).toContain('<details><summary>Technical details</summary>');
    expect(html).not.toContain("<details open");
    const alert = html.match(/<p role="alert"[^>]*>(.*?)<\/p>/)?.[1];
    expect(alert).toBe(message.replaceAll("'", "&#x27;"));
    expect(html).not.toContain('role="status"');
  });
  it("keeps success notices next to the control without error details", () => {
    const html = renderToStaticMarkup(<EditorFeedback value="Scene saved." />);
    expect(html).toContain('role="status"');
    expect(html).toContain("Scene saved.");
    expect(html).not.toContain("<details");
  });
  it("renders nothing after feedback is cleared", () => {
    expect(renderToStaticMarkup(<EditorFeedback value={null} />)).toBe("");
  });
});
