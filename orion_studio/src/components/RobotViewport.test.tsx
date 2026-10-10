import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { projectCatalog } from "../lib/catalog";
import { RobotViewport } from "./RobotViewport";
describe("viewport guidance", () => {
  it.each(["home", "editor"] as const)("uses the same rotation hint in %s", mode => {
    const html = renderToStaticMarkup(<RobotViewport catalog={projectCatalog} joints={projectCatalog.poses.home.positions} light={{ red: 0, green: 0, blue: 0, white: 0 }} mode={mode} />);
    expect(html).toContain("Drag to rotate");
    expect(html).not.toContain("Drag to orbit");
  });
});
