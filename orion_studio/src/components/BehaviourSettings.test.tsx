import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, expect, it, vi } from "vitest";

// Run the subscription effect through visible/hidden lifetimes without a DOM.
// Polling itself (including cancellation and resumption) uses real controllers.
const lifecycle = vi.hoisted(() => ({ effects: [] as Array<() => (() => void) | void> }));
vi.mock("react", async original => ({ ...await original<typeof import("react")>(),
  useEffect: (effect: () => (() => void) | void) => { lifecycle.effects.push(effect); },
}));
vi.mock("../lib/gateway", async original => ({ ...await original<typeof import("../lib/gateway")>(),
  getBehaviours: vi.fn().mockResolvedValue({ behaviours: { mind_idle_rhythm:true, listen_lean_in:true, thinking_beats:true, wake_sleep_acts:true, posture_drift:false, act_sounds:true, object_glances: false, mutual_gaze: false,
    arrival_notice: false, new_object_peek: false, presence_defers_rest: false, proactive_speech: false, demo_mode: false } }),
}));
import { BehaviourSettings } from "./BehaviourSettings";
import { getBehaviours } from "../lib/gateway";

afterEach(() => { lifecycle.effects = []; vi.useRealTimers(); vi.clearAllMocks(); });
it("starts reads only for visible Settings and cancels them when leaving", async () => {
  vi.useFakeTimers();
  const connection = { url: "http://orion", token: "test" };
  const mount = (active: boolean) => {
    renderToStaticMarkup(<BehaviourSettings connection={connection} supported active={active} />);
    const cleanups = lifecycle.effects.splice(0).map(effect => effect());
    return () => cleanups.forEach(cleanup => cleanup?.());
  };
  let unmount = mount(false);
  await vi.advanceTimersByTimeAsync(10_000);
  expect(getBehaviours).not.toHaveBeenCalled();
  unmount(); unmount = mount(true);
  await vi.advanceTimersByTimeAsync(2000);
  expect(getBehaviours).toHaveBeenCalledTimes(2);
  const signal = vi.mocked(getBehaviours).mock.calls[1][1]!;
  unmount(); unmount = mount(false);
  expect(signal.aborted).toBe(true);
  await vi.advanceTimersByTimeAsync(10_000);
  expect(getBehaviours).toHaveBeenCalledTimes(2);
  unmount(); unmount = mount(true);
  await vi.advanceTimersByTimeAsync(0);
  expect(getBehaviours).toHaveBeenCalledTimes(3);
  unmount();
});

it("offers shared-state behaviours and rare sounds", () => {
  const html = renderToStaticMarkup(<BehaviourSettings connection={{url:"http://orion",token:"test"}} supported />);
  for (const text of ["Follow shared energy", "Listening lean", "Thinking beats", "Wake and sleep gestures", "Posture changes", "Rare gesture sounds"]) expect(html).toContain(text);
});
