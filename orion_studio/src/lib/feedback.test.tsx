import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { FailureNotice } from "../components/FailureNotice";
import { failure, forOwner, OwnerRequestError, visibleFeedback } from "./feedback";

describe("page and connection feedback", () => {
  it("clears a disconnected notice when connecting", () => {
    const disconnectedHome = {};
    const notice = { scope: disconnectedHome, value: "Connect Orion to use its controls." };
    expect(visibleFeedback(notice, disconnectedHome)).toContain("Connect Orion");
    const connectedHome = {};
    expect(visibleFeedback(notice, connectedHome)).toBeNull();
  });
  it("clears page-specific notices on navigation, including late responses and revisits", () => {
    const editor = {};
    const home = {};
    const notice = { scope: editor, value: "Scene saved." };
    expect(visibleFeedback(notice, editor)).toBe("Scene saved.");
    expect(visibleFeedback(notice, home)).toBeNull();
    expect(visibleFeedback({ scope: editor, value: "Published to Orion." }, home)).toBeNull();
    expect(visibleFeedback(notice, {})).toBeNull();
  });
  it.each(["Alarm sound saved on Orion.", "Saved. Orion’s next request starts a fresh conversation with these changes.", "Warm white light accepted.", true])("clears local save feedback %s across connections", value => {
    const connection = {};
    const feedback = { scope: connection, value };
    expect(visibleFeedback(feedback, connection)).toBe(value);
    expect(visibleFeedback(feedback, {})).toBeNull();
  });
});

describe("owner-facing failures", () => {
  it("preserves native failures as detail and uses a plain error message", async () => {
    const detail = "IPC load failed: <native diagnostics>";
    const request = forOwner(() => Promise.reject(detail), "Studio couldn't load these settings.");
    await expect(request).rejects.toMatchObject({ message: "Studio couldn't load these settings.", detail });
    const value = failure("Studio couldn't load these settings.", new OwnerRequestError("Friendly", detail));
    const html = renderToStaticMarkup(<FailureNotice value={value} />);
    expect(html).toContain('role="alert"');
    expect(html).toContain("Studio couldn&#x27;t load these settings.");
    expect(html).toContain('<details><summary>Technical details</summary><pre>IPC load failed: &lt;native diagnostics&gt;</pre></details>');
    expect(html).not.toContain("<details open");
  });
  it("passes successful results through without changing them", async () => {
    const value = { muted: true };
    expect(await forOwner(async () => value, "Failed")).toBe(value);
    expect(renderToStaticMarkup(<FailureNotice value={null} />)).toBe("");
  });
});
