export const ORION_SPEECH_SAMPLE_RATE = 24_000;

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
