import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { VolumeControl } from "./VolumeControl";

it.each([0, 40, 100])("renders the lamp's saved volume %s on opening Studio", volume => {
  const html = renderToStaticMarkup(<VolumeControl connection={{url:"http://test",token:"test"}} status={{volume,muted:volume === 0,epoch:"one",revision:0,save_error:null}} />);
  expect(html).toContain(`value="${volume}"`); expect(html).toContain(`${volume}%`);
  expect(html).toContain(volume === 0 ? 'aria-label="Unmute"' : 'aria-label="Mute"');
  expect(html).toContain("Alarms still sound when muted.");
});
it("disables volume when an older lamp does not report it", () => {
  const html = renderToStaticMarkup(<VolumeControl connection={{url:"http://test",token:"test"}} />);
  expect(html).toContain("Update Orion"); expect(html).toMatch(/id="orion-volume"[^>]*disabled=""/);
});
