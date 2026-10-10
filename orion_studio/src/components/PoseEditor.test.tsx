import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { projectCatalog } from "../lib/catalog";
import { PoseEditor } from "./PoseEditor";
describe("scene pose editor", () => {
  it("keeps five calibrated joint controls without standalone authoring fields", () => {
    const html = renderToStaticMarkup(<PoseEditor pose={projectCatalog.poses.home} limits={projectCatalog.jointLimits} onChange={() => {}} />);
    expect(html.match(/type="range"/g)).toHaveLength(5);
    expect(html.match(/type="number"/g)).toHaveLength(5);
    for (const limit of projectCatalog.jointLimits) expect(html).toContain(`min="${limit.lower_rad}" max="${limit.upper_rad}"`);
    for (const text of ["Description", "Tags", "Idle profile", "Default light"]) expect(html).not.toContain(text);
  });
});
