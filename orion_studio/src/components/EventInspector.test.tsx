import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { projectCatalog } from "../lib/catalog";
import { appendSceneMotion } from "../lib/preview";
import { LIGHTING_EFFECTS } from "../types";
import { EventInspector } from "./EventInspector";

describe("scene movement editing", () => {
  it("offers ordering instead of an editable start timestamp", () => {
    const scene = appendSceneMotion(projectCatalog.scenes.acknowledge_left, "next", "acknowledge_nod");
    const html = renderToStaticMarkup(<EventInspector scene={scene} selection={{ track: "motion", id: "next" }} catalog={projectCatalog} markers={[]} onChange={() => {}} onDelete={() => {}} />);
    expect(html).toContain("Movement 2 of 2");
    expect(html).toContain("Move earlier");
    expect(html).toContain("Move later");
    expect(html).not.toContain("Start time");
    expect(html).not.toContain('type="number"');
  });
});


describe("lighting choices", () => {
  it("always exposes generic effects and all Orion presets in separate groups", () => {
    for (const effect of ["pulse", "attentive_focus"] as const) {
      const scene = { ...projectCatalog.scenes.acknowledge_left, lighting: [{ id: "light", effect, at: 0 }] };
      const html = renderToStaticMarkup(<EventInspector scene={scene} selection={{ track: "lighting", id: "light" }} catalog={projectCatalog} markers={[]} onChange={() => {}} onDelete={() => {}} />);
      expect(html).toContain('<optgroup label="Custom effects">');
      expect(html).toContain('<optgroup label="Orion presets">');
      for (const name of [...LIGHTING_EFFECTS,"constant","pulse","breathe","fade"]) expect(html).toContain(`value="${name}"`);
    }
  });
  it("shows percentages for both stage brightness and overall brightness", () => {
    const scene = { ...projectCatalog.scenes.acknowledge_left, lighting: [{ id: "light", effect: "breathe" as const, at: 0, intensity: .42, levels: [.15,.85] }] };
    const html = renderToStaticMarkup(<EventInspector scene={scene} selection={{ track: "lighting", id: "light" }} catalog={projectCatalog} markers={[]} onChange={() => {}} onDelete={() => {}} />);
    for (const percent of [15,85,42]) {
      expect(html).toContain(`<output>${percent}%</output>`);
      expect(html).toContain(`aria-valuetext="${percent}%"`);
    }
  });
});
