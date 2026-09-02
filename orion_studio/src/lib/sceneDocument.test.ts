import { describe, expect, it } from "vitest";

import { buildSceneDocument } from "./sceneDocument";

describe("buildSceneDocument", () => {
  it("strips Studio identities and preserves parallel v2 tracks", () => {
    const document = buildSceneDocument({
      format_version: 2, name: "draft", description: "Parallel", source: "draft",
      motion: [{ id: "motion-ui", at: 0, play: "return_home" }],
      lighting: [{ id: "light-ui", on_marker: "settled", effect: "settle_glow" }],
      audio: [{ id: "audio-ui", at: 0.4, cue: "settle_soft" }],
      finish: { anchor: "final_pose", lighting: "pose_default" },
    }, "published");
    expect(document).toEqual({
      format_version: 2,
      scene: {
        name: "published", description: "Parallel",
        motion: [{ at: 0, play: "return_home" }],
        lighting: [{ on_marker: "settled", effect: "settle_glow" }],
        audio: [{ at: 0.4, cue: "settle_soft" }],
        finish: { anchor: "final_pose", lighting: "pose_default" },
      },
    });
  });
});
