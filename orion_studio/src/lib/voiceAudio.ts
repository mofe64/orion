export const VOICE_SAMPLE_RATE = 16_000;
export const ORION_SPEECH_SAMPLE_RATE = 24_000;
export const WORKER_FRAME_DURATION_MS = 80;
export const WORKER_FRAME_SAMPLES =
  (VOICE_SAMPLE_RATE * WORKER_FRAME_DURATION_MS) / 1_000;

/**
 * Stateful linear resampler for microphone chunks. Keeping fractional position
 * between calls prevents a boundary click or timing drift at chunk edges.
 */
export class StreamingLinearResampler {
  private remainder = new Float32Array(0);
  private position = 0;

  constructor(
    readonly inputRate: number,
    readonly outputRate = VOICE_SAMPLE_RATE,
  ) {
    if (inputRate <= 0 || outputRate <= 0) {
      throw new Error("Audio sample rates must be positive.");
    }
  }

  push(input: Float32Array): Float32Array {
    if (input.length === 0) return new Float32Array(0);
    if (this.inputRate === this.outputRate && this.remainder.length === 0) {
      return input.slice();
    }

    const combined = new Float32Array(this.remainder.length + input.length);
    combined.set(this.remainder);
    combined.set(input, this.remainder.length);

    const step = this.inputRate / this.outputRate;
    const estimated = Math.max(0, Math.ceil((combined.length - 1 - this.position) / step));
    const output = new Float32Array(estimated);
    let count = 0;

    while (this.position + 1 < combined.length) {
      const left = Math.floor(this.position);
      const fraction = this.position - left;
      output[count++] = combined[left] * (1 - fraction) + combined[left + 1] * fraction;
      this.position += step;
    }

    const consumed = Math.min(Math.floor(this.position), combined.length);
    this.remainder = combined.slice(consumed);
    this.position -= consumed;
    return count === output.length ? output : output.slice(0, count);
  }

  reset(): void {
    this.remainder = new Float32Array(0);
    this.position = 0;
  }
}

export function float32ToPcm16(samples: Float32Array): Int16Array {
  const pcm = new Int16Array(samples.length);
  for (let index = 0; index < samples.length; index += 1) {
    const sample = Math.max(-1, Math.min(1, samples[index]));
    pcm[index] = sample < 0 ? Math.round(sample * 32_768) : Math.round(sample * 32_767);
  }
  return pcm;
}

export function resamplePcm16(input: Int16Array, inputRate: number, outputRate = ORION_SPEECH_SAMPLE_RATE): Int16Array {
  if (inputRate === outputRate) return input.slice();
  if (inputRate <= 0 || outputRate <= 0 || input.length === 0) throw new Error("Speech audio requires positive sample rates and samples.");
  const length = Math.max(1, Math.round(input.length * outputRate / inputRate));
  const output = new Int16Array(length);
  const step = inputRate / outputRate;
  for (let index = 0; index < length; index += 1) {
    const position = Math.min(input.length - 1, index * step);
    const left = Math.floor(position);
    const right = Math.min(input.length - 1, left + 1);
    const fraction = position - left;
    output[index] = Math.round(input[left] * (1 - fraction) + input[right] * fraction);
  }
  return output;
}

export function pcm16MonoWav(input: Int16Array, sampleRate: number): Uint8Array {
  const samples = resamplePcm16(input, sampleRate);
  const dataBytes = samples.length * 2;
  const buffer = new ArrayBuffer(44 + dataBytes);
  const view = new DataView(buffer);
  const ascii = (offset: number, value: string) => {
    for (let index = 0; index < value.length; index += 1) view.setUint8(offset + index, value.charCodeAt(index));
  };
  ascii(0, "RIFF");
  view.setUint32(4, 36 + dataBytes, true);
  ascii(8, "WAVE");
  ascii(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, ORION_SPEECH_SAMPLE_RATE, true);
  view.setUint32(28, ORION_SPEECH_SAMPLE_RATE * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  ascii(36, "data");
  view.setUint32(40, dataBytes, true);
  samples.forEach((sample, index) => view.setInt16(44 + index * 2, sample, true));
  return new Uint8Array(buffer);
}

/** Collects arbitrary PCM chunks into fixed 80 ms worker frames. */
export class Pcm16FrameBatcher {
  private pending = new Int16Array(0);

  constructor(readonly frameSamples = WORKER_FRAME_SAMPLES) {
    if (!Number.isInteger(frameSamples) || frameSamples <= 0) {
      throw new Error("Worker frame size must be a positive integer.");
    }
  }

  push(input: Int16Array): Int16Array[] {
    const combined = new Int16Array(this.pending.length + input.length);
    combined.set(this.pending);
    combined.set(input, this.pending.length);

    const frames: Int16Array[] = [];
    let offset = 0;
    while (combined.length - offset >= this.frameSamples) {
      frames.push(combined.slice(offset, offset + this.frameSamples));
      offset += this.frameSamples;
    }
    this.pending = combined.slice(offset);
    return frames;
  }

  reset(): void {
    this.pending = new Int16Array(0);
  }
}

export interface ConditionedAudio {
  pcm: Int16Array;
  workerFrames: Int16Array[];
}

/** Converts native-rate WebView audio into the worker's fixed PCM contract. */
export class VoiceAudioConditioner {
  private readonly batcher = new Pcm16FrameBatcher();
  private resampler: StreamingLinearResampler | null = null;

  accept(input: Float32Array, inputRate: number): ConditionedAudio {
    if (!this.resampler || this.resampler.inputRate !== inputRate) {
      this.resampler = new StreamingLinearResampler(inputRate);
      this.batcher.reset();
    }
    const pcm = float32ToPcm16(this.resampler.push(input));
    return { pcm, workerFrames: this.batcher.push(pcm) };
  }

  reset(): void {
    this.resampler?.reset();
    this.resampler = null;
    this.batcher.reset();
  }
}
