import { afterEach, describe, expect, it, vi } from "vitest";

import {
  getCapabilities,
  getUserScene,
  listUserPoses,
  listUserScenes,
  publishPose,
  publishScene,
  previewScene,
  prepareMovement,
  releaseMovement,
  updateUserScene,
} from "./gateway";

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
        scene_library: { read: true, create: true, update: "revision" },
        joint_limits: [
          { name: "base_yaw_joint", lower_rad: -1.54, upper_rad: 1.54 },
        ],
        pose_library: { read: true, create: true, update: false },
        motion_library: { read: true, create: true, update: false },
        movement_lifecycle: ["prepare", "release"],
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

  it("prepares and releases movement through semantic lifecycle operations", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ api_version: 1, accepted: true }),
    });
    vi.stubGlobal("fetch", fetchMock);
    const connection = { url: "http://orion.local:7447", token: "secret" };

    await prepareMovement(connection);
    await releaseMovement(connection);

    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      "http://orion.local:7447/api/v1/operations",
      expect.objectContaining({ body: JSON.stringify({ operation: "prepare_movement" }) }),
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      "http://orion.local:7447/api/v1/operations",
      expect.objectContaining({ body: JSON.stringify({ operation: "release_movement" }) }),
    );
  });

  it("submits an ephemeral scene preview as a semantic operation", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ api_version: 1, accepted: true }),
    });
    vi.stubGlobal("fetch", fetchMock);
    const connection = { url: "http://orion.local:7447", token: "secret" };
    const document = {
      format_version: 1,
      scene: {
        name: "studio_preview",
        description: "Unsaved preview",
        timeline: [{ at: 0, type: "goto_pose", pose: "home", duration_seconds: 1 }],
      },
    };

    await previewScene(connection, document);

    expect(fetchMock).toHaveBeenCalledWith(
      "http://orion.local:7447/api/v1/operations",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ operation: "preview_scene", document }),
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

  it("loads and revision-updates the Pi user scene library", async () => {
    const revision = "a".repeat(64);
    const changedRevision = "b".repeat(64);
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          api_version: 1,
          scenes: [{
            name: "studio_scene",
            revision,
            bytes: 123,
            relative_path: "scenes/user/studio_scene.yaml",
          }],
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          api_version: 1,
          name: "studio_scene",
          revision,
          relative_path: "scenes/user/studio_scene.yaml",
          yaml: "format_version: 1\nscene:\n  name: studio_scene\n",
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          api_version: 1,
          updated: true,
          name: "studio_scene",
          revision: changedRevision,
          relative_path: "scenes/user/studio_scene.yaml",
        }),
      });
    vi.stubGlobal("fetch", fetchMock);
    const connection = { url: "http://orion.local:7447", token: "secret" };

    await listUserScenes(connection);
    await getUserScene(connection, "studio_scene");
    const document = { format_version: 1, scene: { name: "studio_scene" } };
    await updateUserScene(connection, "studio_scene", revision, document);

    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      "http://orion.local:7447/api/v1/scenes",
      expect.objectContaining({ headers: expect.objectContaining({ Authorization: "Bearer secret" }) }),
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      "http://orion.local:7447/api/v1/scenes/studio_scene",
      expect.objectContaining({ headers: expect.objectContaining({ Authorization: "Bearer secret" }) }),
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      3,
      "http://orion.local:7447/api/v1/scenes/studio_scene",
      expect.objectContaining({
        method: "PUT",
        body: JSON.stringify({ expected_revision: revision, document }),
      }),
    );
  });

  it("lists and creates immutable user keyframe poses", async () => {
    const revision = "c".repeat(64);
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          api_version: 1,
          assets: [{
            name: "studio_pose",
            revision,
            bytes: 200,
            relative_path: "motion/user/poses/studio_pose.yaml",
          }],
        }),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => ({
          api_version: 1,
          published: true,
          already_present: false,
          name: "studio_pose",
          revision,
          relative_path: "motion/user/poses/studio_pose.yaml",
        }),
      });
    vi.stubGlobal("fetch", fetchMock);
    const connection = { url: "http://orion.local:7447", token: "secret" };
    const document = {
      format_version: 1,
      units: "radians",
      poses: { studio_pose: { description: "Test", positions: {} } },
    };

    await listUserPoses(connection);
    await publishPose(connection, document);

    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      "http://orion.local:7447/api/v1/poses",
      expect.objectContaining({ method: "POST", body: JSON.stringify(document) }),
    );
  });
});
