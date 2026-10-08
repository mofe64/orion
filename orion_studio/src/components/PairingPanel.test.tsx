import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PairingController, type ConnectionSnapshot } from "../lib/pairing";
import { PairingPanel } from "./PairingPanel";

describe("saved pairing controls", () => {
  it.each(["connected", "reconnecting", "disconnected"] as const)("allows editing an address while %s", phase => {
    const state: ConnectionSnapshot = {
      phase, persistent: true, paired: true, address: "http://orion.mynet:7447",
      connection: null, status: null, capabilities: null, error: null,
    };
    const html = renderToStaticMarkup(<PairingPanel controller={new PairingController()}
      state={state} onClose={() => {}} />);
    expect(html).toContain('class="quiet-button">Change address</button>');
    expect(html).not.toContain('type="password"');
  });

  it("lists other saved lamps and offers pairing another one", () => {
    const state: ConnectionSnapshot = {
      phase: "connected", persistent: true, paired: true, address: "http://ariadne-robot.local:7447",
      connection: null, status: null, capabilities: null, error: null,
      lamps: [{ url: "http://ariadne-robot.local:7447", active: true }, { url: "http://orion.local:7447", active: false }],
    };
    const html = renderToStaticMarkup(<PairingPanel controller={new PairingController()} state={state} onClose={() => {}} />);
    expect(html).toContain("Switch to orion.local");
    expect(html).not.toContain("Switch to ariadne-robot.local");
    expect(html).toContain("Pair another lamp");
  });
});
