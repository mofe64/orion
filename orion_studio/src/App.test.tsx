import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { ConnectionSnapshot } from "./lib/pairing";

const session = vi.hoisted(() => ({ editor: false, snapshot: { phase: "unpaired", connection: null, status: null, capabilities: null, paired: false } as unknown as ConnectionSnapshot }));
vi.mock("./lib/pairing", () => ({ PairingController: class {
  current = () => session.snapshot;
  subscribe = () => () => {};
} }));
vi.mock("react", async importOriginal => {
  const react = await importOriginal<typeof import("react")>();
  return { ...react, useState: (initial: unknown) => react.useState(session.editor && initial === "home" ? "create" : initial), useSyncExternalStore: (_subscribe: unknown, getSnapshot: () => unknown) => getSnapshot() };
});
vi.mock("./components/RobotViewport", () => ({ RobotViewport: () => null }));
import App from "./App";

describe("connection feedback", () => {
  it("replaces disconnected guidance with the current connection and removes the global notice", () => {
    const offline = renderToStaticMarkup(<App />);
    expect(offline).toContain("Your Orion awaits.");
    expect(offline).toContain(">History</button>");
    expect(offline).not.toContain(">Debug</button>");
    expect(offline).not.toContain("home-theme-toggle");
    expect(offline.match(/class="oh-connect"/g)).toHaveLength(1);
    expect(offline).not.toContain('class="statusbar"');
    session.snapshot = { ...session.snapshot, phase: "connected", connection: { url: "http://test", token: "test" } };
    const online = renderToStaticMarkup(<App />);
    expect(online).toContain("Orion connected");
    expect(online).not.toContain("Your Orion awaits.");
    expect(online).not.toContain("Connect Orion to use voice, character and lamp controls.");
    expect(online).not.toContain("Connected to Orion.");
  });
});

it("shows local save status, one publish action and a plain scene inspector", () => {
  session.editor = true;
  session.snapshot = { ...session.snapshot, phase: "unpaired", connection: null };
  const html = renderToStaticMarkup(<App />);
  expect(html).toContain("Saving…");
  expect(html).toContain('<label for="scene-name">Scene name</label>');
  expect(html).toContain("Connect Orion to publish.");
  expect(html).toContain('aria-label="Selection settings"');
  expect(html).not.toContain('<details class="inspector-panel"');
  expect(html).not.toContain(">Save</button>");
  expect(html).toContain("Preview from");
  expect(html).toContain("Orion starts from wherever it is.");
  const toolbar = html.slice(html.indexOf('class="dock-toolbar"'), html.indexOf('class="timeline"'));
  expect(toolbar).not.toContain("Delay");
  session.editor = false;
});
