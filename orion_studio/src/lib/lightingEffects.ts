import type { LightEvent, LightPreview } from "../types";

export type LightingEffectKind = "pulse" | "breathe";

const MAX_CHANNEL = 255;

function peakColor(baseline: LightPreview): LightPreview {
  return {
    red: Math.min(MAX_CHANNEL, baseline.red + 8),
    green: Math.min(MAX_CHANNEL, baseline.green + 3),
    blue: baseline.blue,
    white: Math.min(MAX_CHANNEL, baseline.white + 48),
  };
}

function lightEvent(
  id: string,
  at: number,
  color: LightPreview,
  transitionSeconds: number,
): LightEvent {
  return {
    id,
    at: Number(at.toFixed(3)),
    type: "light",
    ...color,
    transition_seconds: transitionSeconds,
  };
}

export function createLightingEffect(
  kind: LightingEffectKind,
  at: number,
  baseline: LightPreview,
  idFactory: () => string = () => crypto.randomUUID(),
): LightEvent[] {
  const peak = peakColor(baseline);
  if (kind === "pulse") {
    return [
      lightEvent(idFactory(), at, peak, 0.12),
      lightEvent(idFactory(), at + 0.12, baseline, 0.32),
    ];
  }

  return [0, 2.4].flatMap((offset) => [
    lightEvent(idFactory(), at + offset, peak, 1.2),
    lightEvent(idFactory(), at + offset + 1.2, baseline, 1.2),
  ]);
}
