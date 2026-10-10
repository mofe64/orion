import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { acceptedRun, RunFeedback, runStateLabel } from "./RunFeedback";
import type { GatewayStatus } from "../types";

const connection = { url: "http://test", token: "test" };
const status = { runtime: { motion: null, last_motion: null }, scene: { active: { run_id: 6, name: "acknowledge_left", state: "executing" }, last: null }, speech: { active: null, last: null } } as unknown as GatewayStatus;

describe("Orion activity feedback", () => {
  it("uses human-readable state words and hides internal IDs", () => {
    const html = renderToStaticMarkup(<RunFeedback page="home" status={status} connection={connection} tracked={null} />);
    expect(html).toContain('aria-label="Orion activity"');
    expect(html).toContain("Acknowledge left");
    expect(html).toContain("Playing");
    expect(html).toContain(">Cancel</button>");
    expect(html).not.toContain("run 6");
    expect(html).not.toContain("executing");
  });
  it.each([["queued", "Waiting to start"], ["settling", "Finishing movement"], ["completed", "Finished"], ["cancelled", "Cancelled"], ["failed", "Needs attention"]])("labels %s as %s", (state, label) => {
    expect(runStateLabel(state)).toBe(label);
  });
  it("does not claim a cached activity is still playing after disconnecting", () => {
    const html = renderToStaticMarkup(<RunFeedback page="home" status={status} connection={null} tracked={{ kind: "scene", id: 6, label: "Acknowledge left" }} />);
    expect(html).toContain("Connection lost; completion unknown");
    expect(html).not.toContain(">Cancel</button>");
    expect(html).not.toContain("Playing");
  });
  it("keeps run identifiers for cancellation and result matching", () => {
    expect(acceptedRun({ result: { scene: { run_id: 6 } } }, "scene", "Acknowledge left")).toEqual({ kind: "scene", id: 6, label: "Acknowledge left" });
  });
  it("preserves a confirmed result after the connection is lost", () => {
    const completed = { ...status, scene: { active: null, last: { run_id: 6, state: "completed" } } };
    const html = renderToStaticMarkup(<RunFeedback page="home" status={completed} connection={null} tracked={{ kind: "scene", id: 6, label: "Acknowledge left" }} />);
    expect(html).toContain("Finished");
    expect(html).not.toContain("completion unknown");
  });
});
