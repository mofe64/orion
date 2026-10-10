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
    expect(html).toMatch(/<button class="quiet-button"><svg[^]*?Move earlier/);
    expect(html).toMatch(/<button class="quiet-button" disabled=""><svg[^]*?Move later/);
  });
  it("uses one movement label and the same sentence-case name in the title and dropdown", () => {
    const scene = projectCatalog.scenes.acknowledge_left;
    const html = renderToStaticMarkup(<EventInspector scene={scene} selection={{ track: "motion", id: scene.motion[0].id }} catalog={projectCatalog} markers={[]} onChange={() => {}} onDelete={() => {}} />);
    expect(html).toContain("<h2>Look at left expressive</h2>");
    expect(html).toContain('<option value="look_at_left_expressive" selected="">Look at left expressive</option>');
    expect(html).not.toContain('<p class="eyebrow">Movement</p>');
    expect(html).toContain("<label>Movement<select");
  });
});


describe("lighting choices", () => {
  it("distinguishes stage brightness from the overall multiplier", () => {
    const scene = { ...projectCatalog.scenes.acknowledge_left, lighting: [{ id: "light", effect: "pulse" as const, at: 0 }] };
    const html = renderToStaticMarkup(<EventInspector scene={scene} selection={{ track: "lighting", id: "light" }} catalog={projectCatalog} markers={[]} onChange={() => {}} onDelete={() => {}} />);
    for (const label of ["Base color brightness", "Pulse color brightness", "Overall brightness"]) expect(html).toContain(`aria-label="${label}"`);
    expect(html).toContain("Stage brightness sets each part of the effect. Overall brightness scales them together.");
    expect(html.indexOf("Stage brightness sets")).toBeLessThan(html.indexOf('class="stage-colors"'));
    expect(html).toMatch(/aria-label="Overall brightness" aria-describedby="[^"]+"/);
    expect(html).not.toContain('aria-label="Color brightness"');
  });
  it("always exposes generic effects and all Orion presets in separate groups", () => {
    for (const effect of ["pulse", "attentive_focus"] as const) {
      const scene = { ...projectCatalog.scenes.acknowledge_left, lighting: [{ id: "light", effect, at: 0 }] };
      const html = renderToStaticMarkup(<EventInspector scene={scene} selection={{ track: "lighting", id: "light" }} catalog={projectCatalog} markers={[]} onChange={() => {}} onDelete={() => {}} />);
      expect(html).toContain('<optgroup label="Custom effects">');
      expect(html).toContain('<optgroup label="Orion presets">');
      for (const name of [...LIGHTING_EFFECTS,"constant","pulse","breathe","fade"]) expect(html).toContain(`value="${name}"`);
    }
  });
});

it("offers splitting and delay beside the selected movement", () => {
  const scene = projectCatalog.scenes.acknowledge_left;
  const html = renderToStaticMarkup(<EventInspector scene={scene} selection={{ track: "motion", id: scene.motion[0].id }} catalog={projectCatalog} markers={[]} onChange={() => {}} onDelete={() => {}} onEditPose={() => {}} onSplit={() => {}} onAddDelay={() => {}} />);
  for (const text of ["Split into poses", "Add delay after movement", "Edit destination pose", "Delete movement"]) expect(html).toContain(text);
});

it("displays sound names in sentence case while preserving their stored cue IDs", () => {
  const scene = projectCatalog.scenes.acknowledge_left;
  const event = scene.audio[0];
  const html = renderToStaticMarkup(<EventInspector scene={scene} selection={{ track: "audio", id: event.id }} catalog={projectCatalog} markers={[]} onChange={() => {}} onDelete={() => {}} />);
  expect(html).toContain("<h2>Acknowledge warm</h2>");
  expect(html).toContain('<option value="acknowledge_warm" selected="">Acknowledge warm</option>');
  expect(html).toContain("Delete sound</button>");
  expect(html).not.toContain("Sound event");
});
