import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { LivelyRules, ruleSwitches, usualRules } from "./LivelyRules";

it("shows Lively's rules as plain sentences with a switch for each habit", () => {
  const html = renderToStaticMarkup(<LivelyRules rules={{ ...usualRules, quiet_from: 23 * 60 + 30, sleep_after_goodnight_minutes: null }} pending={false} onChange={() => {}} />);
  expect(html).toContain("Fine-tune Lively");
  expect(html).toContain('value="23:30"');
  expect(html).toContain("After a goodnight, stay awake");
  expect(html).toContain("4 things");
  for (const [, label] of ruleSwitches) expect(html).toContain(label);
  expect(html).not.toMatch(/spacing|per_hour|seconds|lux/);
  expect(html).toContain("Use the usual rules");
});
