import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AnimationLibrary } from "./AnimationLibrary";
import { projectCatalog } from "../lib/catalog";
describe("Animation library structure", () => {
  it("separates browsing and playback from authoring and robot administration", () => {
    const html = renderToStaticMarkup(<AnimationLibrary catalog={projectCatalog} theme="dark" connection={null} status={null} onEdit={() => {}} onRun={() => {}} onNotice={() => {}} />);
    for (const text of ["Orion collection", "My scenes", "Create scene", "Play preview", "Play on Orion", "Return to home pose", "Movement only"]) expect(html).toContain(text);
    for (const text of ["In preview", "Preview at home", "Scene timeline", "Publish asset", "Edit selected event", "Start character", "Diagnostics", "look_at_left_expressive"]) expect(html).not.toContain(text);
  });
});
