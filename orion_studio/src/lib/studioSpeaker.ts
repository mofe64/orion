export interface SpeechPlayer {
  play(pcm: Int16Array, sampleRate: number): Promise<void>;
  stop(): Promise<void>;
}

export class StudioSpeaker implements SpeechPlayer {
  private context: AudioContext | null = null;
  private source: AudioBufferSourceNode | null = null;

  async play(pcm: Int16Array, sampleRate: number): Promise<void> {
    if (pcm.length === 0 || sampleRate <= 0) throw new Error("Speech audio is empty or invalid.");
    if (this.source) throw new Error("Speech playback is already active.");

    const context = this.context ?? new AudioContext();
    this.context = context;
    if (context.state === "suspended") await context.resume();

    const buffer = context.createBuffer(1, pcm.length, sampleRate);
    const channel = buffer.getChannelData(0);
    for (let index = 0; index < pcm.length; index += 1) channel[index] = pcm[index] / 32_768;

    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(context.destination);
    this.source = source;
    await new Promise<void>((resolve) => {
      source.onended = () => {
        if (this.source === source) this.source = null;
        resolve();
      };
      source.start();
    });
  }

  async stop(): Promise<void> {
    const source = this.source;
    this.source = null;
    if (source) {
      source.onended = null;
      source.stop();
      source.disconnect();
    }
    const context = this.context;
    this.context = null;
    if (context && context.state !== "closed") await context.close();
  }
}
