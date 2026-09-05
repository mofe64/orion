import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
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
  it("keeps unknown markers and uncompiled movement out of positioned lanes", () => {
    const html = renderToStaticMarkup(<Timeline scene={scene} trajectories={{}} currentTime={0} selection={null} onSelect={() => {}} onTimeChange={() => {}} />);
    expect(html).toContain("Awaiting compilation");
    expect(html).toContain("attentive focus · marker notice");
    expect(html).toContain("look left · starts at 0.00 s; duration unknown");
    expect(html).not.toContain('aria-label="attentive focus, 0.00 seconds');
    expect(html.match(/class="track-row"/g)).toHaveLength(2);
  });
  it("gives coincident events separate buttons and a full selected label at Fit zoom", () => {
    const html = renderToStaticMarkup(<Timeline scene={scene} trajectories={{}} currentTime={0} selection={{ track: "lighting", id: "two" }} onSelect={() => {}} onTimeChange={() => {}} />);
    expect(html).toContain('aria-label="settle glow, 0.20 seconds"');
    expect(html).toContain('aria-label="acknowledge pulse, 0.20 seconds"');
    expect(html).toContain("lighting · acknowledge pulse · 0.20 s");
    expect(html).toContain('value="1" selected=""');
  });
});
