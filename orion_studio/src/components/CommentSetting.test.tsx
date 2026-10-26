import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { CommentSetting, commentMode, commentPatch } from "./CommentSetting";

it("shows one talk choice and leaving Demo returns to the everyday choice", () => {
  expect(commentMode(false, false)).toBe("off");
  expect(commentMode(true, false)).toBe("sometimes");
  expect(commentMode(true, true)).toBe("demo");
  expect(commentMode(false, true)).toBe("demo");
  expect(commentPatch("demo", true)).toEqual({ demo_mode: true, proactive_speech: true });
  expect(commentPatch("sometimes", false)).toEqual({ demo_mode: false, proactive_speech: true });
  expect(commentPatch("off", true)).toEqual({ demo_mode: false, proactive_speech: false });
  const html = renderToStaticMarkup(<CommentSetting proactive demo={false} onChange={() => {}} />);
  expect(html).toContain("How lively Orion is"); expect(html).toContain("Lively");
  expect(html.match(/checked=""/g)).toHaveLength(1);
  expect(html).not.toMatch(/proactive|PID|rate/i);
});
