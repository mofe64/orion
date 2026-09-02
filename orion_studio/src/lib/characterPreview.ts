const MICRO_IDLE_MIN_SECONDS = 8;
const MICRO_IDLE_MAX_SECONDS = 20;
const LARGE_IDLE_MIN_SECONDS = 35;
const LARGE_IDLE_MAX_SECONDS = 75;

const MICRO_IDLES = [
  "idle_breathe",
  "idle_head_curiosity",
  "idle_micro_glance",
  "idle_shoulder_adjust",
] as const;

const LARGE_IDLES = [
  "idle_weight_shift",
  "idle_soft_head_shake",
  "idle_breathe",
] as const;

export interface SeededIdlePreview {
  category: "micro" | "large";
  clip: string;
  dueSeconds: number;
}

class SeededRandom {
  private state: bigint;

  constructor(seed: number) {
    if (!Number.isFinite(seed)) throw new Error("Idle preview seed must be finite.");
    const normalized = BigInt.asUintN(64, BigInt(Math.trunc(seed)));
    this.state = normalized === 0n ? 1n : normalized;
  }

  next(): bigint {
    let value = this.state;
    value = BigInt.asUintN(64, value ^ (value << 13n));
    value = BigInt.asUintN(64, value ^ (value >> 7n));
    value = BigInt.asUintN(64, value ^ (value << 17n));
    this.state = value;
    return value;
  }

  range(lower: number, upper: number): number {
    const fraction = Number(this.next()) / Number((1n << 64n) - 1n);
    return lower + (upper - lower) * fraction;
  }

  index(length: number): number {
    return Number(this.next() % BigInt(length));
  }
}

/** Mirrors the Pi coordinator's first seeded idle decision for Studio preview. */
export function firstSeededIdle(seed: number, idleProfile?: string): SeededIdlePreview {
  const random = new SeededRandom(seed);
  const microAt = random.range(MICRO_IDLE_MIN_SECONDS, MICRO_IDLE_MAX_SECONDS);
  const largeAt = random.range(LARGE_IDLE_MIN_SECONDS, LARGE_IDLE_MAX_SECONDS);
  const category = microAt <= largeAt ? "micro" : "large";
  const candidates: string[] = category === "micro" ? [...MICRO_IDLES] : [...LARGE_IDLES];
  if (idleProfile === "attentive") candidates.push("idle_attentive_hold");
  if (idleProfile === "directional") candidates.push("idle_directional_hold");
  return {
    category,
    clip: candidates[random.index(candidates.length)],
    dueSeconds: category === "micro" ? microAt : largeAt,
  };
}
