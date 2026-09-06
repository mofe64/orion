import type { LightPreview, SceneLightingEvent } from "../types";
export const CUSTOM_EFFECTS = {
  constant: { label: "Constant", stages: ["Color"], help: "Keeps one color and brightness for the whole event." },
  pulse: { label: "Pulse", stages: ["Base color", "Pulse color"], help: "Briefly flashes the second color, then returns to the base color each cycle." },
  breathe: { label: "Breathe", stages: ["Rest color", "Peak color"], help: "Gradually blends from the rest color to the peak color and back each cycle." },
  fade: { label: "Fade", stages: ["Start color", "End color"], help: "Smoothly changes from the start color to the end color over the event." },
} as const;
export function customLight(event: SceneLightingEvent, time: number): LightPreview | null {
  if (!(event.effect in CUSTOM_EFFECTS)) return null;
  const colors = event.colors ?? ["warm_white", "warm_white"];
  const parse = (value: string) => value === "warm_white" ? [0,0,0,255] : [...[1,3,5].map(index => parseInt(value.slice(index,index+2),16)),0];
  const levels = event.levels ?? (["pulse","breathe"].includes(event.effect) ? [.15,1] : [1,1]);
  const a = parse(colors[0]).map(value => value*(levels[0] ?? 1)), b = parse(colors[1] ?? colors[0]).map(value => value*(levels[1] ?? 1));
  const phase = (time / (event.period ?? 2)) % 1;
  const mix = event.effect === "constant" ? 0 : event.effect === "pulse" ? Math.max(0,1-phase*4) : event.effect === "breathe" ? (1-Math.cos(phase*Math.PI*2))/2 : Math.min(1,time/Math.max(.02,event.duration ?? .8));
  const values = a.map((value,index) => (value+(b[index]-value)*mix)*(event.intensity ?? 1));
  return { red: values[0], green: values[1], blue: values[2], white: values[3] };
}
