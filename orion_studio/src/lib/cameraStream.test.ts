import { afterEach, describe, expect, it, vi } from "vitest";
import { MultipartJpegParser } from "./cameraStream";
import { getCameraSnapshot, setCameraEnabled, watchCamera } from "./gateway";

const type = "multipart/mixed; boundary=orion-frame";
const encode = (value: string) => new TextEncoder().encode(value);
const part = (jpeg = "JPEG", stamp = "12345678901234567") => encode(`--orion-frame\r\nContent-Type: image/jpeg\r\nContent-Length: ${jpeg.length}\r\nX-Capture-Boottime-Ns: ${stamp}\r\n\r\n${jpeg}\r\n`);
const connection = { url: "http://orion/", token: "private-token" };
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

describe("multipart JPEG parser", () => {
  it.each(["mixed", "x-mixed-replace"])("accepts the %s stream type", subtype => {
    const parser = new MultipartJpegParser(`multipart/${subtype}; boundary="orion-frame"`);
    expect(parser.push(part())).toHaveLength(1);
    parser.finish();
  });
  it("handles every possible split in boundaries, headers, images and terminators", () => {
    const wire = part();
    for (let split = 0; split <= wire.length; split++) {
      const parser = new MultipartJpegParser(type);
      const frames = [...parser.push(wire.slice(0, split)), ...parser.push(wire.slice(split))];
      expect(frames).toHaveLength(1);
      expect(frames[0].jpeg).toEqual(encode("JPEG"));
      expect(frames[0].captureBoottimeNs).toBe("12345678901234567");
      parser.finish();
    }
  });
  it("handles single-byte chunks and boundary-like JPEG bytes", () => {
    const parser = new MultipartJpegParser(type);
    const wire = part("JPEG\r\n--orion-frame\r\nbytes");
    const frames = Array.from(wire).flatMap(byte => parser.push(new Uint8Array([byte])));
    expect(frames).toHaveLength(1);
    expect(frames[0].jpeg).toEqual(encode("JPEG\r\n--orion-frame\r\nbytes"));
  });
  it("reads multiple parts and a quoted closing boundary", () => {
    const parser = new MultipartJpegParser('multipart/x-mixed-replace; boundary="orion-frame"');
    const wire = new Uint8Array([...part("ONE"), ...part("TWO"), ...encode("--orion-frame--\r\n")]);
    expect(parser.push(wire).map(frame => new TextDecoder().decode(frame.jpeg))).toEqual(["ONE", "TWO"]);
    parser.finish();
  });
  it("rejects unbounded headers, invalid lengths and truncated images", () => {
    expect(() => new MultipartJpegParser("image/jpeg")).toThrow();
    expect(() => new MultipartJpegParser(type).push(encode("--orion-frame\r\n" + "x".repeat(5000)))).toThrow();
    expect(() => new MultipartJpegParser(type).push(encode("--orion-frame\r\nContent-Type: image/jpeg\r\nContent-Length: 2000000\r\n\r\n"))).toThrow();
    const parser = new MultipartJpegParser(type); parser.push(part().slice(0, -4));
    expect(() => parser.finish()).toThrow("partial frame");
  });
});

describe("authenticated camera transport", () => {
  it("keeps the failure visible throughout the wait and next connection attempt", async () => {
    const controller = new AbortController();
    const states: [string, string | undefined][] = [];
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Camera transport failed")));
    await watchCamera(connection, () => {}, (state, message) => {
      states.push([state, message]);
      if (states.length === 4) controller.abort();
    }, controller.signal, 0);
    expect(states).toEqual([
      ["connecting", undefined],
      ["reconnecting", "Camera transport failed"],
      ["reconnecting", "Camera transport failed"],
      ["reconnecting", "Camera transport failed"],
    ]);
  });
  it("reconnects after EOF and uses a fresh parser for the new stream", async () => {
    const controller = new AbortController();
    const received: string[] = [];
    const states: string[] = [];
    const fetcher = vi.fn()
      .mockResolvedValueOnce(new Response(part("ONE"), { headers: { "Content-Type": type } }))
      .mockResolvedValueOnce(new Response(part("TWO"), { headers: { "Content-Type": type } }));
    vi.stubGlobal("fetch", fetcher);
    await watchCamera(connection, frame => { received.push(new TextDecoder().decode(frame.jpeg)); if (received.length === 2) controller.abort(); },
      state => states.push(state), controller.signal, 0);
    expect(received).toEqual(["ONE", "TWO"]);
    expect(states).toContain("reconnecting");
    expect(fetcher).toHaveBeenCalledTimes(2);
    for (const [url, init] of fetcher.mock.calls) {
      expect(url).toBe("http://orion/api/v2/camera/stream");
      expect(init.headers.Authorization).toBe("Bearer private-token");
      expect(init.redirect).toBe("error");
    }
  });
  it("reconnects after a network error and cancels its body when leaving", async () => {
    const controller = new AbortController(); const cancel = vi.fn();
    const stream = new ReadableStream<Uint8Array>({ start(output) { output.enqueue(part()); }, cancel });
    const fetcher = vi.fn().mockRejectedValueOnce(new TypeError("Connection dropped"))
      .mockResolvedValueOnce(new Response(stream, { headers: { "Content-Type": type } }));
    vi.stubGlobal("fetch", fetcher);
    await watchCamera(connection, () => controller.abort(), () => {}, controller.signal, 0);
    expect(fetcher).toHaveBeenCalledTimes(2);
    expect(cancel).toHaveBeenCalledOnce();
  });
  it("does not reconnect or enable capture after camera_off", async () => {
    const fetcher = vi.fn().mockResolvedValue(new Response('{"error":{"code":"camera_off"}}', { status: 503 }));
    vi.stubGlobal("fetch", fetcher); const state = vi.fn(); const frame = vi.fn();
    await watchCamera(connection, frame, state, new AbortController().signal, 0);
    expect(state).toHaveBeenLastCalledWith("off"); expect(frame).not.toHaveBeenCalled(); expect(fetcher).toHaveBeenCalledOnce();
  });
  it("stops retrying invalid credentials", async () => {
    const fetcher = vi.fn().mockResolvedValue(new Response('{"error":{"code":"unauthorized"}}', { status: 401 }));
    vi.stubGlobal("fetch", fetcher); const state = vi.fn();
    await watchCamera(connection, () => {}, state, new AbortController().signal, 0);
    expect(state).toHaveBeenLastCalledWith("unavailable", "Reconnect Orion to view its camera.");
    expect(fetcher).toHaveBeenCalledOnce();
  });
  it("authenticates snapshots and sends a strict privacy Boolean", async () => {
    const fetcher = vi.fn().mockResolvedValueOnce(new Response("JPEG", { headers: { "Content-Type": "image/jpeg" } }))
      .mockResolvedValueOnce(new Response('{"camera_enabled":false}'));
    vi.stubGlobal("fetch", fetcher);
    expect(await (await getCameraSnapshot(connection)).text()).toBe("JPEG");
    expect(await setCameraEnabled(connection, false)).toEqual({ camera_enabled: false });
    expect(fetcher.mock.calls[1][1].body).toBe('{"camera_enabled":false}');
    expect(fetcher.mock.calls[0][1].headers.Authorization).toBe("Bearer private-token");
  });
});
