import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
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
  it("groups split pose and delay clips on one collapsed Motion track, even offline", () => {
    const definition = structuredClone(projectCatalog.motions.look_at_left_expressive);
    definition.keyframes[3].hold = .4;
    const catalog = { ...projectCatalog, motions: { ...projectCatalog.motions, [definition.name]: definition } };
    const splitScene = { ...scene, motion: [{ ...scene.motion[0], play: definition.name, show_parts: true }] };
    const html = renderToStaticMarkup(<Timeline scene={splitScene} catalog={catalog} trajectories={{}} currentTime={0} selection={{ track: "motion", id: "move", component: { index: 1, kind: "pose" } }} onSelect={() => {}} onTimeChange={() => {}} onToggleParts={() => {}} onChangeMovement={() => {}} />);
    expect(html).not.toContain("look at left expressive");
    expect(html).not.toContain('aria-label="Poses and delays"');
    expect(html.match(/aria-label="pose:/g)).toHaveLength(4);
    expect(html.match(/aria-label="delay:/g)).toHaveLength(1);
    expect(html).toContain('aria-label="Motion track"');
    expect(html).toContain('aria-expanded="false"');
    expect(html).not.toContain('id="motion-track-components"');
    expect(html).toContain("pose · look left lean");
    expect(html).not.toContain("Movement options");
  });

  it("keeps unknown markers and uncompiled movement out of positioned lanes", () => {
    const html = renderToStaticMarkup(<Timeline scene={scene} trajectories={{}} currentTime={0} selection={null} onSelect={() => {}} onTimeChange={() => {}} />);
    expect(html).toContain("Calculating scene timing");
    expect(html).toContain("attentive focus · linked to notice");
    expect(html).toContain("look left");
    expect(html).not.toContain('aria-label="attentive focus, 0.00 seconds');
    expect(html.match(/class="track-row motion-group-header"/g)).toHaveLength(3);
  });
  it("gives coincident events separate buttons and a full selected label at Fit zoom", () => {
    const html = renderToStaticMarkup(<Timeline scene={scene} trajectories={{}} currentTime={0} selection={{ track: "lighting", id: "two" }} onSelect={() => {}} onTimeChange={() => {}} />);
    expect(html).toContain('aria-label="lighting: settle glow, 0.20 seconds"');
    expect(html).toContain('aria-label="lighting: acknowledge pulse, 0.20 seconds"');
    expect(html).toContain("lighting · acknowledge pulse · 0.20 s");
    expect(html).toContain('value="1" selected=""');
  });
});
