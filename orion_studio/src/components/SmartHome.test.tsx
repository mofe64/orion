import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { SmartHome, HomeRooms } from "./SmartHome";
import { HomeFlowForm, fieldKind } from "./HomeFlow";
import { homeUrl, integrationList, type HomeDevices, type HomeFlow } from "../lib/home";
const devices: HomeDevices = { areas: [{ name: "Study", entities: [{ entity_id: "light.desk", name: "Desk lamp", area: "Study", domain: "light", state: "off", exposed: true, attributes: {} }] }], devices: [{ id: "device_1", name: "Desk lamp" }], area_registry: [{ area_id: "study", name: "Study" }] };
const noop = () => {};
const form = (flow: HomeFlow) => renderToStaticMarkup(<HomeFlowForm flow={flow} url="http://pi:8123/" devices={devices} busy={false} onSubmit={noop} onAbort={noop} onRefresh={noop}/>);
describe("Home", () => {
  it("renders rooms, device state, simple control and assistant exposure", () => {
    expect(renderToStaticMarkup(<SmartHome connection={null}/>)).toContain("Connect Orion");
    const html = renderToStaticMarkup(<HomeRooms devices={devices} busy={false} control={noop} expose={noop}/>);
    for (const label of ["Study", "Desk lamp", "Turn on", "Orion can control", "checked"]) expect(html).toContain(label);
  });
  it("renders native flow fields and common selectors", () => {
    const html = form({ type: "form", flow_id: "setup", data_schema: [
      { name: "host", type: "string", required: true }, { name: "password", selector: { text: { type: "password" } } },
      { name: "port", type: "integer", default: 8123 }, { name: "enabled", type: "boolean" },
      { name: "mode", type: "select", options: [["one", "First"], ["two", "Second"]] },
      { name: "room", selector: { area: {} } }, { name: "target", selector: { entity: { domain: "light" } } },
      { name: "device", selector: { device: {} } }, { name: "time", selector: { time: {} } },
    ] });
    for (const label of ['type="password"', 'type="number"', 'type="checkbox"', 'type="time"', "First", "Study", "light.desk", "device_1", "Continue"]) expect(html).toContain(label);
    expect(html).not.toContain("Continue in Home Assistant");
    expect(fieldKind({ name: "x", selector: { target: {} } })).toBe("unsupported");
  });
  it("provides full Home Assistant continuation for unsupported steps and fields", () => {
    expect(form({ type: "form", data_schema: [{ name: "target", selector: { target: {} } }] })).toContain('href="http://pi:8123/"');
    expect(form({ type: "external" })).toContain("Continue in Home Assistant");
    expect(form({ type: "menu", menu_options: ["manual_setup"] })).toContain("manual setup");
    expect(form({ type: "create_entry" })).toContain("Device added.");
  });
  it("uses the connected Pi hostname and searches normalized integration names", () => {
    expect(homeUrl({ url: "http://[::1]:7447", token: "studio" })).toBe("http://[::1]:8123/");
    expect(integrationList(["hue", { domain: "matter", name: "Matter" }])).toEqual([{ domain: "hue", name: "hue" }, { domain: "matter", name: "Matter" }]);
  });
});
