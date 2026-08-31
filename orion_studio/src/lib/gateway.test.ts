import { afterEach, describe, expect, it, vi } from "vitest";

import { getCapabilities, publishScene } from "./gateway";

describe("Studio gateway client", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("fetches the versioned semantic capability catalog with bearer authentication", async () => {
    const capabilities = {
      api_version: 1,
      capabilities: {
        goto: ["home"],
        motion: ["wave"],
        scene: ["hello"],
        speech: { max_text_bytes: 1024 },
        cancel: ["movement", "scene", "speech"],
        scene_publish: { format_version: 1, max_body_bytes: 262144 },
      },
    };
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => capabilities,
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(getCapabilities({
      url: "http://orion.local:7447/",
      token: "secret",
    })).resolves.toEqual(capabilities);

    expect(fetchMock).toHaveBeenCalledWith(
      "http://orion.local:7447/api/v1/capabilities",
      expect.objectContaining({
        headers: expect.objectContaining({ Authorization: "Bearer secret" }),
      }),
    );
  });

  it("publishes a versioned scene with bearer authentication", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        api_version: 1,
        published: true,
        already_present: false,
        name: "studio_scene",
        relative_path: "scenes/user/studio_scene.yaml",
      }),
    });
    vi.stubGlobal("fetch", fetchMock);
    const document = {
      format_version: 1,
      scene: { name: "studio_scene", description: "Test", timeline: [] },
    };

    await publishScene({ url: "http://orion.local:7447/", token: "secret" }, document);

    expect(fetchMock).toHaveBeenCalledWith(
      "http://orion.local:7447/api/v1/scenes",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify(document),
        headers: expect.objectContaining({ Authorization: "Bearer secret" }),
      }),
    );
  });

  it("surfaces the gateway's refusal message", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({
      ok: false,
      status: 409,
      json: async () => ({ error: { message: "A different user scene already exists." } }),
    }));

    await expect(publishScene(
      { url: "http://orion.local:7447", token: "secret" },
      {},
    )).rejects.toThrow("A different user scene already exists.");
  });
});
