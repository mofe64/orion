import type {
  VoiceAudioFrame,
  VoiceCaptureInfo,
  VoiceCaptureSource,
} from "./voiceRuntime";

const WORKLET_NAME = "orion-studio-microphone";
const DEFAULT_WORKLET_URL = "/orion-microphone-worklet.js";

export interface StudioMicrophoneDependencies {
  mediaDevices?: MediaDevices;
  createAudioContext?: () => AudioContext;
  workletUrl?: string;
  now?: () => number;
}

function abortError(): Error {
  const error = new Error("Microphone start was cancelled.");
  error.name = "AbortError";
  return error;
}

/** Browser/Tauri WebView adapter. Raw frames remain transient and in memory. */
export class StudioMicrophone implements VoiceCaptureSource {
  private stream: MediaStream | null = null;
  private context: AudioContext | null = null;
  private sourceNode: MediaStreamAudioSourceNode | null = null;
  private workletNode: AudioWorkletNode | null = null;
  private mutedOutput: GainNode | null = null;
  private generation = 0;
  private sequence = 0;

  constructor(private readonly dependencies: StudioMicrophoneDependencies = {}) {}

  async start(
    onFrame: (frame: VoiceAudioFrame) => void,
    onError: (error: unknown) => void,
  ): Promise<VoiceCaptureInfo> {
    if (this.stream || this.context) throw new Error("Studio microphone capture is already active.");
    const mediaDevices = this.dependencies.mediaDevices ?? navigator.mediaDevices;
    if (!mediaDevices?.getUserMedia) {
      throw new Error("This platform does not expose microphone capture to Orion Studio.");
    }

    const generation = ++this.generation;
    this.sequence = 0;
    try {
      const stream = await mediaDevices.getUserMedia({
        audio: {
          channelCount: { ideal: 1 },
          sampleRate: { ideal: 48_000 },
          echoCancellation: false,
          noiseSuppression: false,
          autoGainControl: false,
        },
        video: false,
      });
      if (generation !== this.generation) {
        for (const track of stream.getTracks()) track.stop();
        throw abortError();
      }
      this.stream = stream;

      const context = this.dependencies.createAudioContext?.()
        ?? new AudioContext({ latencyHint: "interactive" });
      this.context = context;
      await context.audioWorklet.addModule(
        this.dependencies.workletUrl ?? DEFAULT_WORKLET_URL,
      );
      if (generation !== this.generation) throw abortError();

      const sourceNode = context.createMediaStreamSource(stream);
      const workletNode = new AudioWorkletNode(context, WORKLET_NAME, {
        numberOfInputs: 1,
        numberOfOutputs: 1,
        outputChannelCount: [1],
        processorOptions: { frameDurationMs: 20 },
      });
      const mutedOutput = context.createGain();
      mutedOutput.gain.value = 0;
      workletNode.port.onmessage = (event: MessageEvent<unknown>) => {
        if (generation !== this.generation || !(event.data instanceof Float32Array)) return;
        onFrame({
          samples: event.data,
          sampleRate: context.sampleRate,
          channels: 1,
          sequence: this.sequence++,
          capturedAtMs: (this.dependencies.now ?? performance.now.bind(performance))(),
        });
      };
      workletNode.port.onmessageerror = onError;
      sourceNode.connect(workletNode);
      workletNode.connect(mutedOutput);
      mutedOutput.connect(context.destination);
      this.sourceNode = sourceNode;
      this.workletNode = workletNode;
      this.mutedOutput = mutedOutput;
      await context.resume();

      const track = stream.getAudioTracks()[0];
      return {
        deviceLabel: track?.label || "System default microphone",
        sampleRate: context.sampleRate,
        channels: 1,
      };
    } catch (error) {
      if (generation === this.generation) await this.releaseResources();
      throw error;
    }
  }

  async stop(): Promise<void> {
    ++this.generation;
    await this.releaseResources();
  }

  private async releaseResources(): Promise<void> {
    if (this.workletNode) {
      this.workletNode.port.onmessage = null;
      this.workletNode.port.onmessageerror = null;
      this.workletNode.disconnect();
      this.workletNode = null;
    }
    this.sourceNode?.disconnect();
    this.sourceNode = null;
    this.mutedOutput?.disconnect();
    this.mutedOutput = null;
    if (this.stream) {
      for (const track of this.stream.getTracks()) track.stop();
      this.stream = null;
    }
    const context = this.context;
    this.context = null;
    if (context && context.state !== "closed") await context.close();
  }
}
