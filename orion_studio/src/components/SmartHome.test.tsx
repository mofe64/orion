import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { SmartHome, HomeRooms } from "./SmartHome";
import { HomeFlowForm, fieldKind } from "./HomeFlow";
import { PasswordInput } from "./HomeInputs";
import { homeUrl, homeText, integrationList, stateLabel, optimisticEntity, type HomeDevices, type HomeFlow, type HomeEntity } from "../lib/home";
const lamp: HomeEntity = { entity_id: "light.desk", name: "Desk lamp", area: "Study", domain: "light", state: "off", exposed: true, attributes: { brightness: 128, supported_color_modes: ["brightness"] } };
const devices: HomeDevices = { areas: [{ name: "Study", entities: [lamp] }], devices: [{ id: "device_1", name: "Desk lamp", area: "Study", area_id: "study", entities: [lamp] }], area_registry: [{ area_id: "study", name: "Study" }] };
const noop = () => {};
const form = (flow: HomeFlow) => renderToStaticMarkup(<HomeFlowForm flow={flow} url="http://pi:8123/" devices={devices} busy={false} onSubmit={noop} onAbort={noop} onRefresh={noop}/>);
describe("Home", () => {
  it("groups physical devices with room selection, manual controls and labelled voice switches", () => {
    expect(renderToStaticMarkup(<SmartHome connection={null}/>)).toContain("Connect Orion");
    const html = renderToStaticMarkup(<HomeRooms devices={devices} busy={false} control={noop} expose={noop} room={noop}/>);
    for (const label of ["Study", "Desk lamp", "Turn on", "Voice control", 'role="switch"', 'aria-checked="true"', "Room for Desk lamp", "Brightness for Desk lamp", "Let Orion control this with your voice."]) expect(html).toContain(label);
    expect(html).not.toContain('type="checkbox"');
    expect(html).not.toContain("light.desk");
  });
  it("renders password visibility controls for legacy and selector password fields", () => {
    const html = form({ type: "form", data_schema: [{ name: "password", selector: { text: { type: "password" } } }, { name: "account_password", type: "password" }] });
    expect(html.match(/type="password"/g)).toHaveLength(2);
    expect(html.match(/aria-pressed="false"/g)).toHaveLength(2);
    expect(html).toContain('aria-label="Show account password"');
    expect(renderToStaticMarkup(<PasswordInput label="Password" required/>)).toContain('aria-controls=');
  });
  it("renders supported setup fields without internal labels", () => {
    const html = form({ type: "form", flow_id: "setup", data_schema: [
      { name: "host", type: "string", required: true }, { name: "password", selector: { text: { type: "password" } } },
      { name: "port", type: "integer", default: 8123 }, { name: "enabled", type: "boolean" },
      { name: "mode", type: "select", options: [["one", "First"], ["two", "Second"]] },
      { name: "room", selector: { area: {} } }, { name: "target", selector: { entity: { domain: "light" } } },
      { name: "device", selector: { device: {} } }, { name: "time", selector: { time: {} } },
    ] });
    for (const label of ['type="password"', 'type="number"', 'type="checkbox"', 'type="time"', "First", "Study", "Device address", "Continue"]) expect(html).toContain(label);
    expect(html).not.toContain("Continue in Home Assistant");
    expect(fieldKind({ name: "x", selector: { target: {} } })).toBe("unsupported");
  });
  it("provides continuation, retry and cancel paths for setup steps", () => {
    expect(form({ type: "external" })).toContain('href="http://pi:8123/"');
    expect(form({ type: "external" })).toContain("Check setup");
    expect(form({ type: "menu", menu_options: ["manual_setup"] })).toContain("Enter device details");
    expect(form({ type: "create_entry" })).toContain("Choose a room");
    expect(form({ type: "abort", reason: "already_configured" })).toContain("already in your home");
    const error = form({ type: "form", errors: { base: "cannot_connect", api_key: "invalid_auth" } });
    expect(error).toContain("Could not connect."); expect(error).not.toContain("cannot_connect"); expect(error).not.toContain("base:");
  });
  it("supports control types independently of voice permission and restricts unsafe devices", () => {
    const entries = [lamp, { ...lamp, entity_id: "climate.heat", domain: "climate", attributes: { temperature: 21 } }, { ...lamp, entity_id: "cover.blind", domain: "cover", attributes: { supported_features: 11 } }, { ...lamp, entity_id: "media_player.tv", domain: "media_player", state: "playing", attributes: { supported_features: 16389, volume_level: .3 } }, { ...lamp, entity_id: "cover.garage", domain: "cover", attributes: { device_class: "garage" } }];
    const data = { ...devices, devices: entries.map((entity, i) => ({ id: String(i), name: `Device ${i}`, area: "Study", entities: [{ ...entity, exposed: false }] })) };
    const html = renderToStaticMarkup(<HomeRooms devices={data} busy={false} control={noop} expose={noop} room={noop}/>);
    for (const label of ["Turn on", "Target temperature", "Open", "Close", "Stop", "Pause", "Volume for", "For safety, use the device’s own app"]) expect(html).toContain(label);
    expect(html.match(/role="switch"/g)).toHaveLength(4);
    expect(html).not.toContain('aria-label="Open Desk lamp" disabled');
  });
  it("formats states and optimistic settings without exposing machine strings", () => {
    expect(stateLabel(lamp)).toBe("Off");
    expect(stateLabel({ ...lamp, state: "unavailable" })).toBe("Not responding");
    expect(stateLabel({ ...lamp, domain: "sensor", state: "21.00", attributes: { unit_of_measurement: "°C" } })).toBe("21 °C");
    expect(stateLabel({ ...lamp, state: "2026-10-18T00:00:00Z" })).toBe("State unavailable");
    expect(optimisticEntity(lamp, { command: "on", brightness: 80 })).toMatchObject({ state: "on", attributes: { brightness: 204 } });
    expect(optimisticEntity(lamp, { command: "volume", volume: 60 }).attributes.volume_level).toBe(.6);
    expect(homeText("api_key")).toBe("Access key");
    expect(homeText("light.desk")).toBe("Device");
  });
  it("uses the lamp address and normalizes brand names", () => {
    expect(homeUrl({ url: "http://ariadne-robot.local:7447", token: "studio" })).toBe("http://ariadne-robot.local:8123/");
    expect(homeUrl({ url: "http://[::1]:7447", token: "studio" })).toBe("http://[::1]:8123/");
    expect(() => homeUrl({ url: "file:///tmp", token: "studio" })).toThrow();
    expect(integrationList(["hue", { domain: "matter", name: "Matter" }])).toEqual([{ domain: "hue", name: "Philips Hue" }, { domain: "matter", name: "Matter" }]);
  });
});
