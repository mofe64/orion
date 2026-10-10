import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { projectCatalog } from "../lib/catalog";
import { EventInspector } from "./EventInspector";
import { updateMovementPart } from "../lib/movementComponents";
describe("movement parts", () => {
  it("reveals all four poses and exposes pauses only at settled positions", () => {
    const motion = projectCatalog.motions.look_at_left_expressive;
    const scene = { ...projectCatalog.scenes.acknowledge_left, motion: [{ id: "move", at: 0, play: motion.name }] };
    const html = motion.keyframes.map((_, index) => renderToStaticMarkup(<EventInspector scene={scene} catalog={projectCatalog} selection={{ track: "motion", id: "move", component: { index, kind: "pose" } }} markers={[]} onChange={() => {}} onDelete={() => {}} onChangeMovement={() => {}} />)).join("");
    expect((html.match(/Move duration \(s\)/g) ?? []).length).toBe(4);
    expect((html.match(/Continues smoothly/g) ?? []).length).toBe(3);
    expect((html.match(/Pause after \(s\)/g) ?? []).length).toBe(1);
  });
  it("editing a part preserves markers, style, smooth joins and the system original", () => {
    const motion = projectCatalog.motions.look_at_left_expressive;
    const before = structuredClone(motion);
    const edited = updateMovementPart(motion, 1, { duration: 0.6 });
    expect(edited.keyframes[1].duration).toBe(.6);
    expect(edited.keyframes[1].marker).toBe("notice");
    expect(edited.keyframes[1].arrival).toBe("through");
    expect(edited.style).toBe(motion.style);
    expect(motion).toEqual(before);
  });
});
