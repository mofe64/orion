const view = vi.hoisted(() => ({ expanded: false }));
vi.mock("react", async importOriginal => {
  const react = await importOriginal<typeof import("react")>();
  return { ...react, useState: (initial: unknown) => react.useState(view.expanded && initial !== null && typeof initial === "object" && Object.keys(initial).length === 0 ? { motion: true } : initial) };
});
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { projectCatalog } from "../lib/catalog";
import { Timeline } from "./Timeline";
import type { SceneDefinition } from "../types";

const scene: SceneDefinition = {
  format_version: 2, name: "timeline", description: "", source: "draft",
  motion: [{ id: "move", at: 0, play: "look_left" }],
  lighting: [
    { id: "pending", on_marker: "notice", effect: "attentive_focus" },
    { id: "one", at: 0.2, duration: 1, effect: "settle_glow" },
    { id: "two", at: 0.2, duration: 1, effect: "acknowledge_pulse" },
  ],
  audio: [], finish: { anchor: "final_pose", lighting: "pose_default" },
};

describe("timeline uncertainty and overlapping events", () => {
  it("groups split pose and delay clips on one collapsed Movement track, even offline", () => {
    const definition = structuredClone(projectCatalog.motions.look_at_left_expressive);
    definition.keyframes[3].hold = .4;
    const catalog = { ...projectCatalog, motions: { ...projectCatalog.motions, [definition.name]: definition } };
    const splitScene = { ...scene, motion: [{ ...scene.motion[0], play: definition.name, show_parts: true }] };
    const html = renderToStaticMarkup(<Timeline scene={splitScene} catalog={catalog} trajectories={{}} currentTime={0} selection={{ track: "motion", id: "move", component: { index: 1, kind: "pose" } }} onSelect={() => {}} onTimeChange={() => {}} onToggleParts={() => {}} onChangeMovement={() => {}} />);
    expect(html).not.toContain("Look at left expressive");
    expect(html).not.toContain('aria-label="Poses and delays"');
    expect(html.match(/aria-label="Pose:/g)).toHaveLength(4);
    expect(html.match(/aria-label="Delay:/g)).toHaveLength(1);
    expect(html).toContain('aria-label="Movement track"');
    expect(html).toContain('aria-expanded="false"');
    expect(html).not.toContain('id="motion-track-components"');
    expect(html).toContain("Pose · Look left lean");
    expect(html).not.toContain("Movement options");
    expect(html).toContain("1 movement");
    expect(html).not.toContain("5 components");
  });

  it("keeps unknown markers and uncompiled movement out of positioned lanes", () => {
    const html = renderToStaticMarkup(<Timeline scene={scene} trajectories={{}} currentTime={0} selection={null} onSelect={() => {}} onTimeChange={() => {}} />);
    expect(html).toContain("Calculating scene timing");
    expect(html).toContain("Orion must calculate their durations before placing linked light and sound.");
    expect(html).not.toContain("Connect Orion to calculate");
    expect(html).toContain("Attentive focus · linked to notice");
    expect(html).toContain("Look left");
    expect(html).not.toContain('aria-label="attentive focus, 0.00 seconds');
    expect(html.match(/class="track-row motion-group-header"/g)).toHaveLength(3);
  });
  it("gives coincident events separate buttons and a full selected label at Fit zoom", () => {
    const html = renderToStaticMarkup(<Timeline scene={scene} trajectories={{}} currentTime={0} selection={{ track: "lighting", id: "two" }} onSelect={() => {}} onTimeChange={() => {}} />);
    expect(html).toContain('aria-label="Light: Settle glow, 0.20 seconds"');
    expect(html).toContain('aria-label="Light: Acknowledge pulse, 0.20 seconds"');
    expect(html).toContain('title="Light: Acknowledge pulse"');
    expect(html).toContain("Light · Acknowledge pulse · 0.20 s");
    expect(html).toContain('value="1" selected=""');
    expect(html).toContain("3 light cues");
    expect(html).toContain("0 sounds");
  });
});

it("keeps the expanded track summary populated alongside its detailed rows", () => {
  view.expanded = true;
  const html = renderToStaticMarkup(<Timeline scene={scene} trajectories={{}} currentTime={0} selection={null} onSelect={() => {}} onTimeChange={() => {}} />);
  const header = html.slice(html.indexOf('class="track-row motion-group-header"'), html.indexOf('id="motion-track-components"'));
  expect(header).toContain('aria-expanded="true"');
  expect(header).toContain('aria-label="Movement: Look left, 0.00 seconds"');
  expect(html).toContain('id="motion-track-components"');
  expect(html.indexOf('aria-label="Sound track"')).toBeLessThan(html.indexOf('id="motion-track-components"'));
  expect(html).toContain('aria-label="Movement items"');
  expect(html.indexOf('class="timeline-details"')).toBeGreaterThan(html.indexOf('aria-label="Sound track"'));
  expect(html.indexOf('class="timeline-details"')).toBeLessThan(html.indexOf('id="motion-track-components"'));
  expect(html).toContain('aria-label="Light track"');
  expect(html).toContain('aria-label="Sound track"');
  view.expanded = false;
});
