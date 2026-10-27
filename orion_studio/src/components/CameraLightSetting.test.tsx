import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { CameraLightSetting, lightPoint, nearestLight } from "./CameraLightSetting";

it("finds the light under a point on the drawn ring", () => {
  for (let index = 0; index < 24; index++) {
    const { x, y } = lightPoint(index, 24);
    expect(nearestLight(x, y, 24)).toBe(index);
    expect(nearestLight(x + 3, y - 2, 24)).toBe(index);
  }
  expect(lightPoint(0, 24).y).toBeLessThan(lightPoint(12, 24).y);
});

it("offers to place the camera light in plain words", () => {
  const html = renderToStaticMarkup(<CameraLightSetting connection={null} />);
  expect(html).toContain("Camera light");
  expect(html).toContain("Place the light");
  expect(html).not.toMatch(/\bpixels?\b|\bLEDs?\b|\blease\b|\bindex\b/i);
});
