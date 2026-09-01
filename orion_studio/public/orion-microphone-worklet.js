class OrionStudioMicrophoneProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const duration = options.processorOptions?.frameDurationMs ?? 20;
    this.frameSamples = Math.max(1, Math.round(sampleRate * duration / 1000));
    this.pending = new Float32Array(this.frameSamples);
    this.pendingLength = 0;
  }

  process(inputs) {
    const input = inputs[0]?.[0];
    if (!input) return true;

    let sourceOffset = 0;
    while (sourceOffset < input.length) {
      const available = this.frameSamples - this.pendingLength;
      const copied = Math.min(available, input.length - sourceOffset);
      this.pending.set(input.subarray(sourceOffset, sourceOffset + copied), this.pendingLength);
      this.pendingLength += copied;
      sourceOffset += copied;

      if (this.pendingLength === this.frameSamples) {
        const frame = this.pending;
        this.port.postMessage(frame, [frame.buffer]);
        this.pending = new Float32Array(this.frameSamples);
        this.pendingLength = 0;
      }
    }
    return true;
  }
}

registerProcessor("orion-studio-microphone", OrionStudioMicrophoneProcessor);
