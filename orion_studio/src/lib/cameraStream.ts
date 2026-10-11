export interface CameraFrame {
  jpeg: Uint8Array;
  captureBoottimeNs: string | null;
}

const HEADER_LIMIT = 4096;
const FRAME_LIMIT = 1024 * 1024;
const bytes = (value: string) => new TextEncoder().encode(value);
function find(buffer: Uint8Array, target: Uint8Array): number {
  outer: for (let i = 0; i <= buffer.length - target.length; i++) {
    for (let j = 0; j < target.length; j++) if (buffer[i + j] !== target[j]) continue outer;
    return i;
  }
  return -1;
}

/** Content-Length delimits JPEG bytes, so a boundary-like sequence inside an
 * image cannot split it. Headers and images may arrive over arbitrary chunks. */
export class MultipartJpegParser {
  private buffer = new Uint8Array(0);
  private boundary: Uint8Array;
  private part: { length: number; stamp: string | null } | null = null;
  private ended = false;

  constructor(contentType: string) {
    const match = /boundary=(?:"([^"]+)"|([^;\s]+))/i.exec(contentType);
    const boundary = match?.[1] ?? match?.[2];
    if (!/^multipart\/(?:mixed|x-mixed-replace)\s*;/i.test(contentType) || !boundary || !/^[\w.-]{1,70}$/.test(boundary)) {
      throw new Error("Camera stream has no valid multipart boundary.");
    }
    this.boundary = bytes(`--${boundary}`);
  }

  push(chunk: Uint8Array): CameraFrame[] {
    if (this.ended) return [];
    if (this.buffer.length + chunk.length > FRAME_LIMIT + HEADER_LIMIT + 128) throw new Error("Camera part exceeds its size limit.");
    const joined = new Uint8Array(this.buffer.length + chunk.length);
    joined.set(this.buffer); joined.set(chunk, this.buffer.length); this.buffer = joined;
    const frames: CameraFrame[] = [];
    while (true) {
      if (this.part) {
        const { length, stamp } = this.part;
        if (this.buffer.length < length + 2) break;
        if (this.buffer[length] !== 13 || this.buffer[length + 1] !== 10) throw new Error("Camera part has no terminator.");
        frames.push({ jpeg: this.buffer.slice(0, length), captureBoottimeNs: stamp });
        this.buffer = this.buffer.slice(length + 2); this.part = null;
        continue;
      }
      const start = find(this.buffer, this.boundary);
      if (start < 0) {
        if (this.buffer.length > HEADER_LIMIT) throw new Error("Camera boundary was not found.");
        break;
      }
      if (start) this.buffer = this.buffer.slice(start);
      if (this.buffer.length < this.boundary.length + 2) break;
      if (this.buffer[this.boundary.length] === 45 && this.buffer[this.boundary.length + 1] === 45) {
        this.ended = true; this.buffer = new Uint8Array(0); break;
      }
      const end = find(this.buffer, bytes("\r\n\r\n"));
      if (end < 0) {
        if (this.buffer.length > HEADER_LIMIT) throw new Error("Camera headers exceed their size limit.");
        break;
      }
      if (end > HEADER_LIMIT) throw new Error("Camera headers exceed their size limit.");
      const header = new TextDecoder().decode(this.buffer.slice(this.boundary.length, end));
      const lengthText = /(?:^|\r\n)Content-Length:\s*(\d+)\s*(?:\r\n|$)/i.exec(header)?.[1];
      const length = Number(lengthText);
      if (!lengthText || !Number.isSafeInteger(length) || length <= 0 || length > FRAME_LIMIT ||
          !/(?:^|\r\n)Content-Type:\s*image\/jpeg\s*(?:\r\n|$)/i.test(header)) {
        throw new Error("Camera part has invalid JPEG headers.");
      }
      this.part = { length, stamp: /X-Capture-Boottime-Ns:\s*(\d+)/i.exec(header)?.[1] ?? null };
      this.buffer = this.buffer.slice(end + 4);
    }
    return frames;
  }

  finish() {
    if (this.part || this.buffer.length) throw new Error("Camera stream ended in a partial frame.");
  }
}
