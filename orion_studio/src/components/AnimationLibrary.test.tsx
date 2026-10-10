import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { AnimationLibrary, DELETE_SCENE_FAILURE, deleteSceneWithNotice } from "./AnimationLibrary";
import { projectCatalog } from "../lib/catalog";
describe("Animation library structure", () => {
  it("separates browsing and playback from authoring and robot administration", () => {
    const html = renderToStaticMarkup(<AnimationLibrary catalog={projectCatalog} theme="dark" connection={null} status={null} onEdit={() => {}} onRun={() => {}} />);
    for (const text of ["Orion collection", "My scenes", "Create scene", "Preview", "Play on Orion", "Return to home pose", "Movement only"]) expect(html).toContain(text);
    expect(html).toContain("Movement only plays the movement without light or sound.");
    expect(html).not.toContain(">Return home</span>");
    for (const text of ["In preview", "Preview at home", "Scene timeline", "Publish asset", "Edit selected event", "Start character", "Diagnostics", "look_at_left_expressive"]) expect(html).not.toContain(text);
  });
});

describe("scene deletion confirmation", () => {
  it("shows confirmation only after deletion completes", async () => {
    let finish!: () => void;
    const remove = vi.fn(() => new Promise<void>(resolve => { finish = resolve; }));
    const notice = vi.fn();
    const pending = deleteSceneWithNotice(remove, notice);
    expect(notice).not.toHaveBeenCalled();
    finish(); await pending;
    expect(remove).toHaveBeenCalledOnce();
    expect(notice).toHaveBeenCalledWith("Scene deleted.");
  });
  it("does not claim success when deletion fails", async () => {
    const notice = vi.fn();
    await expect(deleteSceneWithNotice(async () => { throw new Error("revision conflict"); }, notice)).rejects.toMatchObject({ message: DELETE_SCENE_FAILURE, detail: "revision conflict" });
    expect(notice).not.toHaveBeenCalled();
  });
});

it("uses sentence-case scene names and owner descriptions without decorative eyebrows", () => {
  const html = renderToStaticMarkup(<AnimationLibrary catalog={projectCatalog} theme="dark" connection={null} status={null} onEdit={() => {}} onRun={() => {}} />);
  expect(html).toContain("Acknowledge left");
  expect(html).toContain("Turn left with a warm light and a short sound.");
  expect(html).not.toContain("marker-synchronised");
  expect(html).not.toContain("ORION STUDIO");
  expect(html).not.toContain("MAKE IT YOURS");
});
