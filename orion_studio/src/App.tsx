import { invoke } from "@tauri-apps/api/core";
import { load as loadYaml } from "js-yaml";
import {
  CircleStop,
  CloudCog,
  Lightbulb,
  Link2,
  Move3d,
  Music2,
  Plus,
  Power,
  Radio,
  Save,
  Sparkles,
  Unplug,
  Volume2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { EventInspector } from "./components/EventInspector";
import { MotionEditor } from "./components/MotionEditor";
import { PoseEditor } from "./components/PoseEditor";
import { RobotViewport } from "./components/RobotViewport";
import { Timeline } from "./components/Timeline";
import { projectCatalog } from "./lib/catalog";
import { createLightingEffect, type LightingEffectKind } from "./lib/lightingEffects";
import {
  cancelRun,
  getCapabilities,
  getUserMotion,
  getUserPose,
  getUserScene,
  getStatus,
  gotoPose,
  listUserScenes,
  listUserMotions,
  listUserPoses,
  publishPose,
  publishMotion,
  publishScene,
  prepareMovement,
  releaseMovement,
  runMotion,
  runScene,
  updateUserScene,
  type GatewayConnection,
  type UserAssetSource,
  type UserSceneSource,
} from "./lib/gateway";
import { sampleSceneLight, sampleScenePose, sceneDuration } from "./lib/preview";
import type {
  GatewayCapabilities,
  GatewayStatus,
  JointName,
  MotionDefinition,
  MotionKeyframe,
  PoseDefinition,
  SceneDefinition,
  SceneEvent,
  StoredSceneDocument,
  StoredMotionDocument,
  StoredPoseDocument,
} from "./types";

type PreviewKind = "scene" | "pose" | "motion";

function copyScene(scene: SceneDefinition): SceneDefinition {
  return structuredClone(scene);
}

function sortedTimeline(events: SceneEvent[]): SceneEvent[] {
  return [...events].sort((left, right) => left.at - right.at);
}

function sceneDocument(scene: SceneDefinition, name: string) {
  return {
    format_version: 1,
    scene: {
      name,
      description: scene.description,
      timeline: scene.timeline.map(({ id: _id, ...event }) => event),
    },
  };
}

function poseDocument(pose: PoseDefinition, name: string): StoredPoseDocument {
  return {
    format_version: 1,
    units: "radians",
    poses: {
      [name]: {
        description: pose.description,
        positions: pose.positions,
      },
    },
  };
}

function motionDocument(motion: MotionDefinition, name: string): StoredMotionDocument {
  return {
    format_version: 1,
    motion: {
      name,
      description: motion.description,
      keyframes: motion.keyframes,
    },
  };
}

function storedScene(document: StoredSceneDocument, remoteRevision?: string): SceneDefinition {
  return {
    format_version: 1,
    name: document.scene.name,
    description: document.scene.description,
    source: "user",
    remote_revision: remoteRevision,
    timeline: document.scene.timeline.map((event) => ({
      ...event,
      id: crypto.randomUUID(),
    })) as SceneEvent[],
  };
}

function remoteScene(source: UserSceneSource): SceneDefinition {
  const document = loadYaml(source.yaml) as StoredSceneDocument;
  if (
    !document
    || document.format_version !== 1
    || !document.scene
    || document.scene.name !== source.name
    || !Array.isArray(document.scene.timeline)
  ) {
    throw new Error(`Pi user scene '${source.name}' does not match its library metadata.`);
  }
  return storedScene(document, source.revision);
}

function remotePose(source: UserAssetSource): PoseDefinition {
  const document = loadYaml(source.yaml) as StoredPoseDocument;
  const entry = document?.poses?.[source.name];
  if (
    document?.format_version !== 1
    || document.units !== "radians"
    || Object.keys(document.poses ?? {}).length !== 1
    || !entry
  ) {
    throw new Error(`Pi user pose '${source.name}' does not match its library metadata.`);
  }
  return {
    name: source.name,
    description: entry.description,
    positions: entry.positions,
    source: "user",
    remote_revision: source.revision,
  };
}

function storedPose(document: StoredPoseDocument): PoseDefinition {
  const [name, entry] = Object.entries(document.poses)[0] ?? [];
  if (!name || !entry) throw new Error("Local user pose document is empty.");
  return {
    name,
    description: entry.description,
    positions: entry.positions,
    source: "user",
  };
}

function remoteMotion(source: UserAssetSource): MotionDefinition {
  const document = loadYaml(source.yaml) as StoredMotionDocument;
  if (document?.format_version !== 1 || document.motion?.name !== source.name) {
    throw new Error(`Pi user motion '${source.name}' does not match its library metadata.`);
  }
  return {
    ...document.motion,
    source: "user",
    remote_revision: source.revision,
  };
}

function storedMotion(document: StoredMotionDocument): MotionDefinition {
  return {
    ...document.motion,
    source: "user",
  };
}

function createEvent(type: SceneEvent["type"], at: number, catalog = projectCatalog): SceneEvent {
  const id = crypto.randomUUID();
  switch (type) {
    case "play_motion":
      return { id, at, type, motion: Object.keys(catalog.motions)[0] };
    case "goto_pose":
      return { id, at, type, pose: Object.keys(catalog.poses)[0], duration_seconds: 2 };
    case "light":
      return { id, at, type, red: 8, green: 3, blue: 0, white: 20, transition_seconds: 0.35 };
    case "audio":
      return { id, at, type, cue: catalog.cues[0] ?? "acknowledge" };
  }
}

function runLabel(run: { state: string; name?: string; run_id: number } | null | undefined): string {
  if (!run) return "No active run";
  return `${run.name ?? "run"} · ${run.state} · #${run.run_id}`;
}

export default function App() {
  const firstScene = projectCatalog.scenes.acknowledge_left ?? Object.values(projectCatalog.scenes)[0];
  const [sceneCatalog, setSceneCatalog] = useState(projectCatalog.scenes);
  const [poseCatalog, setPoseCatalog] = useState(projectCatalog.poses);
  const [motionCatalog, setMotionCatalog] = useState(projectCatalog.motions);
  const [jointLimits, setJointLimits] = useState(projectCatalog.jointLimits);
  const [scene, setScene] = useState<SceneDefinition>(() => copyScene(firstScene));
  const [previewKind, setPreviewKind] = useState<PreviewKind>("scene");
  const [poseDraft, setPoseDraft] = useState<PoseDefinition | null>(null);
  const [poseSaveAsName, setPoseSaveAsName] = useState("my_keyframe");
  const [poseDirty, setPoseDirty] = useState(false);
  const [motionDraft, setMotionDraft] = useState<MotionDefinition | null>(null);
  const [motionSaveAsName, setMotionSaveAsName] = useState("my_motion");
  const [motionDirty, setMotionDirty] = useState(false);
  const [selectedLibraryScene, setSelectedLibraryScene] = useState(firstScene.name);
  const [selectedEventId, setSelectedEventId] = useState<string | null>(scene.timeline[0]?.id ?? null);
  const [currentTime, setCurrentTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [saveAsName, setSaveAsName] = useState(`${firstScene.name}_studio`);
  const [projectRoot, setProjectRoot] = useState("");
  const [notice, setNotice] = useState("Built-in assets loaded from the Orion project.");

  const [connectionOpen, setConnectionOpen] = useState(false);
  const [gatewayUrl, setGatewayUrl] = useState(() => localStorage.getItem("orionStudioGateway") ?? "http://orion.local:7447");
  const [gatewayToken, setGatewayToken] = useState(() => sessionStorage.getItem("orionStudioToken") ?? "");
  const [connected, setConnected] = useState(false);
  const [robotStatus, setRobotStatus] = useState<GatewayStatus | null>(null);
  const [robotCapabilities, setRobotCapabilities] = useState<GatewayCapabilities | null>(null);
  const [cueDurations, setCueDurations] = useState<Record<string, number>>({});
  const [previewCue, setPreviewCue] = useState<string | null>(null);
  const playbackFrame = useRef(0);
  const previewAudio = useRef<HTMLAudioElement | null>(null);
  const previewAudioQueue = useRef<string[]>([]);
  const dispatchedPreviewCues = useRef(new Set<string>());
  const catalog = useMemo(
    () => ({
      ...projectCatalog,
      poses: poseCatalog,
      motions: motionCatalog,
      scenes: sceneCatalog,
      jointLimits,
    }),
    [sceneCatalog, poseCatalog, motionCatalog, jointLimits],
  );
  const previewCatalog = useMemo(() => {
    if (previewKind !== "motion" || !motionDraft) return catalog;
    const event = scene.timeline.find((item) => item.type === "play_motion");
    if (event?.type !== "play_motion") return catalog;
    return {
      ...catalog,
      motions: { ...catalog.motions, [event.motion]: motionDraft },
    };
  }, [catalog, motionDraft, previewKind, scene.timeline]);

  const duration = useMemo(() => {
    const cueEnd = scene.timeline.reduce((latest, event) => {
      if (event.type !== "audio") return latest;
      return Math.max(latest, event.at + (cueDurations[event.cue] ?? 1));
    }, 0);
    return Math.max(sceneDuration(scene, previewCatalog), cueEnd);
  }, [scene, cueDurations, previewCatalog]);
  const sampledJoints = useMemo(
    () => sampleScenePose(scene, previewCatalog, currentTime),
    [scene, previewCatalog, currentTime],
  );
  const previewJoints = previewKind === "pose" && poseDraft
    ? poseDraft.positions
    : sampledJoints;
  const previewLight = useMemo(() => sampleSceneLight(scene, currentTime), [scene, currentTime]);
  const selectedEvent = scene.timeline.find((event) => event.id === selectedEventId) ?? null;
  const connection: GatewayConnection = { url: gatewayUrl, token: gatewayToken };
  const activeRobotRun = robotStatus?.scene.active ?? robotStatus?.runtime.motion;
  const latestRobotRun = activeRobotRun ?? robotStatus?.scene.last ?? robotStatus?.runtime.last_motion;
  const robotRunSummary = activeRobotRun
    ? `Active: ${runLabel(activeRobotRun)}`
    : latestRobotRun
      ? `Last: ${runLabel(latestRobotRun)}`
      : "No runs yet";

  const stopPreviewAudio = useCallback(() => {
    previewAudioQueue.current = [];
    if (previewAudio.current) {
      previewAudio.current.pause();
      previewAudio.current.currentTime = 0;
      previewAudio.current = null;
    }
    setPreviewCue(null);
  }, []);

  const playNextPreviewCue = useCallback(function playNextPreviewCue() {
    if (previewAudio.current) return;
    const cue = previewAudioQueue.current.shift();
    if (!cue) return;
    const url = catalog.cueUrls[cue];
    if (!url) return playNextPreviewCue();
    const audio = new Audio(url);
    previewAudio.current = audio;
    setPreviewCue(cue);
    const finished = () => {
      if (previewAudio.current === audio) previewAudio.current = null;
      setPreviewCue(null);
      playNextPreviewCue();
    };
    audio.addEventListener("ended", finished, { once: true });
    audio.addEventListener("error", finished, { once: true });
    void audio.play().catch(finished);
  }, [catalog.cueUrls]);

  const resetPreviewCues = (time: number) => {
    stopPreviewAudio();
    dispatchedPreviewCues.current.clear();
    for (const event of scene.timeline) {
      if (event.type === "audio" && event.at < time) {
        dispatchedPreviewCues.current.add(event.id);
      }
    }
  };

  useEffect(() => {
    invoke<string>("default_project_root")
      .then((root) => {
        setProjectRoot(root);
        invoke<StoredSceneDocument[]>("load_user_scenes", { projectRoot: root })
          .then((documents) => {
            setSceneCatalog((current) => {
              const next = { ...current };
              for (const document of documents) {
                if (next[document.scene.name]?.source === "built_in") continue;
                next[document.scene.name] = storedScene(document);
              }
              return next;
            });
          })
          .catch((error) => setNotice(`Could not load user scenes: ${String(error)}`));
        invoke<StoredPoseDocument[]>("load_user_poses", { projectRoot: root })
          .then((documents) => {
            setPoseCatalog((current) => {
              const next = { ...current };
              for (const document of documents) {
                const pose = storedPose(document);
                if (next[pose.name]?.source !== "built_in") next[pose.name] = pose;
              }
              return next;
            });
          })
          .catch((error) => setNotice(`Could not load user poses: ${String(error)}`));
        invoke<StoredMotionDocument[]>("load_user_motions", { projectRoot: root })
          .then((documents) => {
            setMotionCatalog((current) => {
              const next = { ...current };
              for (const document of documents) {
                const motion = storedMotion(document);
                if (next[motion.name]?.source !== "built_in") next[motion.name] = motion;
              }
              return next;
            });
          })
          .catch((error) => setNotice(`Could not load user motions: ${String(error)}`));
      })
      .catch(() => {
        setProjectRoot("");
        setNotice("No local Orion checkout was found. Connect to the Pi, or set ORION_PROJECT_ROOT for offline staging.");
      });
  }, []);

  useEffect(() => {
    const audioElements: HTMLAudioElement[] = [];
    for (const [cue, url] of Object.entries(catalog.cueUrls)) {
      const audio = new Audio(url);
      const loaded = () => {
        if (Number.isFinite(audio.duration)) {
          setCueDurations((current) => ({ ...current, [cue]: audio.duration }));
        }
      };
      audio.addEventListener("loadedmetadata", loaded);
      audioElements.push(audio);
    }
    return () => {
      for (const audio of audioElements) audio.src = "";
    };
  }, [catalog.cueUrls]);

  useEffect(() => {
    cancelAnimationFrame(playbackFrame.current);
    if (!playing) return;
    const startedAt = performance.now() - currentTime * 1000;
    const update = (now: number) => {
      const next = (now - startedAt) / 1000;
      if (next >= duration) {
        setCurrentTime(duration);
        setPlaying(false);
        return;
      }
      setCurrentTime(next);
      playbackFrame.current = requestAnimationFrame(update);
    };
    playbackFrame.current = requestAnimationFrame(update);
    return () => cancelAnimationFrame(playbackFrame.current);
  }, [playing, duration]);

  useEffect(() => {
    if (!playing) return;
    for (const event of scene.timeline) {
      if (
        event.type === "audio"
        && event.at <= currentTime
        && !dispatchedPreviewCues.current.has(event.id)
      ) {
        dispatchedPreviewCues.current.add(event.id);
        previewAudioQueue.current.push(event.cue);
      }
    }
    playNextPreviewCue();
  }, [currentTime, playing, scene.timeline, playNextPreviewCue]);

  useEffect(() => {
    if (!playing) stopPreviewAudio();
  }, [playing, stopPreviewAudio]);

  useEffect(() => {
    stopPreviewAudio();
    dispatchedPreviewCues.current.clear();
  }, [scene.name, stopPreviewAudio]);

  useEffect(() => {
    if (!connected) return;
    const poll = window.setInterval(() => {
      getStatus(connection)
        .then(setRobotStatus)
        .catch((error) => {
          setConnected(false);
          setNotice(`Robot connection lost: ${error.message}`);
        });
    }, 750);
    return () => window.clearInterval(poll);
  }, [connected, gatewayUrl, gatewayToken]);

  const loadScene = (name: string) => {
    resetPreviewCues(0);
    const next = copyScene(catalog.scenes[name]);
    setScene(next);
    setPoseDraft(null);
    setPoseDirty(false);
    setMotionDraft(null);
    setMotionDirty(false);
    setPreviewKind("scene");
    setSelectedLibraryScene(name);
    setSelectedEventId(next.timeline[0]?.id ?? null);
    setCurrentTime(0);
    setPlaying(false);
    setDirty(false);
    setSaveAsName(`${name}_studio`);
    setNotice(next.source === "built_in"
      ? `Loaded built-in scene “${name}” as a read-only source. Save As creates a user copy.`
      : next.remote_revision
        ? `Loaded Pi user scene “${name}”. Save changes uses revision checking; Save As creates a copy.`
        : `Loaded local user scene “${name}”. Save As creates another local or Pi copy.`);
  };

  const previewPose = (name: string) => {
    resetPreviewCues(0);
    const next: SceneDefinition = {
      format_version: 1,
      name: `${name}_pose_preview`,
      description: `Studio preview of the ${name} pose.`,
      source: "draft",
      timeline: [{ id: crypto.randomUUID(), at: 0, type: "goto_pose", pose: name, duration_seconds: 1 }],
    };
    setPoseDraft(structuredClone(catalog.poses[name]));
    setPoseSaveAsName(
      connected && catalog.poses[name].source === "user" && !catalog.poses[name].remote_revision
        ? name
        : `${name}_keyframe`,
    );
    setPoseDirty(false);
    setMotionDraft(null);
    setMotionDirty(false);
    setScene(next);
    setPreviewKind("pose");
    setSelectedLibraryScene("");
    setSelectedEventId(next.timeline[0].id);
    setCurrentTime(1);
    setDirty(false);
    setSaveAsName(`${name}_scene`);
    setNotice(`Viewing named pose “${name}”.`);
  };

  const newPose = () => {
    resetPreviewCues(0);
    const nextPose: PoseDefinition = {
      name: "untitled_keyframe",
      description: "A new Orion Studio keyframe pose.",
      positions: structuredClone(previewJoints),
      source: "user",
    };
    const preview: SceneDefinition = {
      format_version: 1,
      name: "untitled_keyframe_preview",
      description: "Unsaved Studio keyframe preview.",
      source: "draft",
      timeline: [{
        id: crypto.randomUUID(),
        at: 0,
        type: "goto_pose",
        pose: Object.keys(catalog.poses)[0],
        duration_seconds: 1,
      }],
    };
    setPoseDraft(nextPose);
    setPoseSaveAsName("my_keyframe");
    setPoseDirty(true);
    setMotionDraft(null);
    setMotionDirty(false);
    setScene(preview);
    setPreviewKind("pose");
    setSelectedLibraryScene("");
    setSelectedEventId(preview.timeline[0].id);
    setCurrentTime(1);
    setPlaying(false);
    setDirty(false);
    setNotice("Created a pose draft from the current preview. Joint controls do not move Orion.");
  };

  const previewMotion = (name: string) => {
    resetPreviewCues(0);
    const next: SceneDefinition = {
      format_version: 1,
      name: `${name}_motion_preview`,
      description: `Studio preview of the ${name} motion.`,
      source: "draft",
      timeline: [{ id: crypto.randomUUID(), at: 0, type: "play_motion", motion: name }],
    };
    setScene(next);
    setPoseDraft(null);
    setPoseDirty(false);
    setMotionDraft(structuredClone(catalog.motions[name]));
    setMotionSaveAsName(
      connected && catalog.motions[name].source === "user" && !catalog.motions[name].remote_revision
        ? name
        : `${name}_studio`,
    );
    setMotionDirty(false);
    setPreviewKind("motion");
    setSelectedLibraryScene("");
    setSelectedEventId(next.timeline[0].id);
    setCurrentTime(0);
    setPlaying(false);
    setDirty(false);
    setSaveAsName(`${name}_scene`);
    setNotice(`Viewing authored motion “${name}” with Studio’s quintic preview sampler.`);
  };

  const newMotion = () => {
    resetPreviewCues(0);
    const initialPose = poseDraft && catalog.poses[poseDraft.name]
      ? poseDraft.name
      : Object.keys(catalog.poses)[0];
    const draft: MotionDefinition = {
      name: "untitled_motion",
      description: "A new Orion Studio keyframe motion.",
      keyframes: [{ pose: initialPose, duration: 1.5, hold: 0 }],
      source: "user",
    };
    const preview: SceneDefinition = {
      format_version: 1,
      name: "untitled_motion_preview",
      description: "Unsaved Studio motion preview.",
      source: "draft",
      timeline: [{
        id: crypto.randomUUID(),
        at: 0,
        type: "play_motion",
        motion: draft.name,
      }],
    };
    setMotionDraft(draft);
    setMotionSaveAsName("my_motion");
    setMotionDirty(true);
    setPoseDraft(null);
    setPoseDirty(false);
    setScene(preview);
    setPreviewKind("motion");
    setSelectedLibraryScene("");
    setSelectedEventId(preview.timeline[0].id);
    setCurrentTime(0);
    setPlaying(false);
    setDirty(false);
    setNotice("Created a motion draft. Add named pose keyframes; Orion retains quintic interpolation.");
  };

  const newScene = () => {
    resetPreviewCues(0);
    const next: SceneDefinition = {
      format_version: 1,
      name: "untitled_scene",
      description: "A new Orion Studio scene.",
      source: "draft",
      timeline: [createEvent("light", 0, catalog)],
    };
    setScene(next);
    setPoseDraft(null);
    setPoseDirty(false);
    setMotionDraft(null);
    setMotionDirty(false);
    setPreviewKind("scene");
    setSelectedLibraryScene("");
    setSelectedEventId(next.timeline[0].id);
    setCurrentTime(0);
    setDirty(true);
    setSaveAsName("my_orion_scene");
    setNotice("Created a new scene draft. Add semantic clips, then Save As to scenes/user/.");
  };

  const updateEvent = (updated: SceneEvent) => {
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline(current.timeline.map((event) => event.id === updated.id ? updated : event)),
    }));
    setDirty(true);
  };

  const moveEvent = (id: string, at: number) => {
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline(current.timeline.map((event) => event.id === id ? { ...event, at } : event)),
    }));
    setCurrentTime(at);
    setPlaying(false);
    setDirty(true);
  };

  const updateSceneDescription = (description: string) => {
    setScene((current) => ({ ...current, description, source: "draft" }));
    setDirty(true);
  };

  const deleteEvent = () => {
    if (!selectedEventId || scene.timeline.length === 1) {
      setNotice("A scene must keep at least one event.");
      return;
    }
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: current.timeline.filter((event) => event.id !== selectedEventId),
    }));
    setSelectedEventId(null);
    setDirty(true);
  };

  const addEvent = (type: SceneEvent["type"]) => {
    const event = createEvent(type, Number(currentTime.toFixed(2)), catalog);
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline([...current.timeline, event]),
    }));
    setSelectedEventId(event.id);
    setDirty(true);
  };

  const addLightingEffect = (kind: LightingEffectKind) => {
    const events = createLightingEffect(kind, Number(currentTime.toFixed(3)), previewLight);
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline([...current.timeline, ...events]),
    }));
    setSelectedEventId(events[0].id);
    setDirty(true);
    setNotice(
      `${kind === "pulse" ? "Pulse" : "Breathe"} added as ${events.length} editable RGBW fades using the existing scene format.`,
    );
  };

  const updatePosePosition = (joint: JointName, value: number) => {
    const limit = jointLimits.find((item) => item.name === joint);
    if (!poseDraft || !limit || !Number.isFinite(value)) return;
    const bounded = Math.max(limit.lower_rad, Math.min(limit.upper_rad, value));
    setPoseDraft((current) => current ? {
      ...current,
      positions: { ...current.positions, [joint]: bounded },
    } : current);
    setPoseDirty(true);
  };

  const updateMotionKeyframes = (keyframes: MotionKeyframe[]) => {
    setMotionDraft((current) => current ? { ...current, keyframes } : current);
    setMotionDirty(true);
  };

  const savePoseDraft = async () => {
    if (!poseDraft) return;
    if (!connected && !projectRoot) {
      setNotice("Connect to Orion to save on the Pi, or set ORION_PROJECT_ROOT for offline staging.");
      return;
    }
    const name = poseSaveAsName.trim();
    if (!/^[A-Za-z0-9_-]+$/.test(name)) {
      setNotice("Pose name may contain only letters, numbers, underscores, and hyphens.");
      return;
    }
    const document = poseDocument(poseDraft, name);
    try {
      const result = connected
        ? await publishPose(connection, document)
        : await invoke<{ name: string; relative_path: string }>("save_user_pose", {
          projectRoot,
          document,
        });
      const saved: PoseDefinition = {
        ...structuredClone(poseDraft),
        name: result.name,
        source: "user",
        remote_revision: "revision" in result && typeof result.revision === "string"
          ? result.revision
          : undefined,
      };
      const preview: SceneDefinition = {
        format_version: 1,
        name: `${saved.name}_pose_preview`,
        description: `Studio preview of the ${saved.name} pose.`,
        source: "draft",
        timeline: [{
          id: crypto.randomUUID(),
          at: 0,
          type: "goto_pose",
          pose: saved.name,
          duration_seconds: 1,
        }],
      };
      setPoseCatalog((current) => ({ ...current, [saved.name]: saved }));
      setPoseDraft(saved);
      setPoseSaveAsName(`${saved.name}_v2`);
      setPoseDirty(false);
      setScene(preview);
      setSelectedEventId(preview.timeline[0].id);
      setNotice(`Saved immutable keyframe ${result.relative_path}${connected ? " on Orion" : " in local staging"}.`);
    } catch (error) {
      setNotice(`Could not save pose: ${String(error)}`);
    }
  };

  const saveMotionDraft = async () => {
    if (!motionDraft) return;
    if (!connected && !projectRoot) {
      setNotice("Connect to Orion to save on the Pi, or set ORION_PROJECT_ROOT for offline staging.");
      return;
    }
    const name = motionSaveAsName.trim();
    if (!/^[A-Za-z0-9_-]+$/.test(name)) {
      setNotice("Motion name may contain only letters, numbers, underscores, and hyphens.");
      return;
    }
    if (motionDraft.keyframes.some((frame) => (
      !catalog.poses[frame.pose]
      || !Number.isFinite(frame.duration)
      || frame.duration <= 0
      || !Number.isFinite(frame.hold)
      || frame.hold < 0
    ))) {
      setNotice("Every keyframe needs a known pose, a positive transition, and a non-negative hold.");
      return;
    }
    const document = motionDocument(motionDraft, name);
    try {
      const result = connected
        ? await publishMotion(connection, document)
        : await invoke<{ name: string; relative_path: string }>("save_user_motion", {
          projectRoot,
          document,
        });
      const saved: MotionDefinition = {
        ...structuredClone(motionDraft),
        name: result.name,
        source: "user",
        remote_revision: "revision" in result && typeof result.revision === "string"
          ? result.revision
          : undefined,
      };
      const preview: SceneDefinition = {
        format_version: 1,
        name: `${saved.name}_motion_preview`,
        description: `Studio preview of the ${saved.name} motion.`,
        source: "draft",
        timeline: [{
          id: crypto.randomUUID(),
          at: 0,
          type: "play_motion",
          motion: saved.name,
        }],
      };
      setMotionCatalog((current) => ({ ...current, [saved.name]: saved }));
      setMotionDraft(saved);
      setMotionSaveAsName(`${saved.name}_v2`);
      setMotionDirty(false);
      setScene(preview);
      setSelectedEventId(preview.timeline[0].id);
      setNotice(`Saved immutable motion ${result.relative_path}${connected ? " on Orion" : " in local staging"}.`);
    } catch (error) {
      setNotice(`Could not save motion: ${String(error)}`);
    }
  };

  const saveUserScene = async () => {
    if (!connected && !projectRoot) {
      setNotice("Connect to Orion to save on the Pi, or set ORION_PROJECT_ROOT for offline staging.");
      return;
    }
    const name = saveAsName.trim();
    if (!/^[A-Za-z0-9_-]+$/.test(name)) {
      setNotice("Save As name may contain only letters, numbers, underscores, and hyphens.");
      return;
    }
    const document = sceneDocument(scene, name);
    try {
      const result = connected
        ? await publishScene(connection, document)
        : await invoke<{ name: string; relative_path: string }>("save_user_scene", {
          projectRoot,
          document,
        });
      const savedScene: SceneDefinition = {
        ...copyScene(scene),
        name: result.name,
        source: "user",
        remote_revision: "revision" in result && typeof result.revision === "string"
          ? result.revision
          : undefined,
      };
      setScene(savedScene);
      setSceneCatalog((current) => ({ ...current, [savedScene.name]: savedScene }));
      setSelectedLibraryScene(savedScene.name);
      setDirty(false);
      setSaveAsName(`${savedScene.name}_copy`);
      setNotice(
        `Saved ${result.relative_path}${connected ? " on Orion" : " in the local project"}. Studio did not modify any built-in scene.`,
      );
    } catch (error) {
      setNotice(`Could not save scene: ${String(error)}`);
    }
  };

  const saveSceneChanges = async () => {
    if (!connected || !scene.remote_revision) {
      setNotice("Connect to Orion and load a Pi user scene before saving changes in place.");
      return;
    }
    try {
      const result = await updateUserScene(
        connection,
        scene.name,
        scene.remote_revision,
        sceneDocument(scene, scene.name),
      );
      const savedScene: SceneDefinition = {
        ...copyScene(scene),
        source: "user",
        remote_revision: result.revision,
      };
      setScene(savedScene);
      setSceneCatalog((current) => ({ ...current, [savedScene.name]: savedScene }));
      setDirty(false);
      setNotice(`Saved revision-checked changes to ${result.relative_path} and reloaded Orion's scene catalog.`);
    } catch (error) {
      setNotice(`Could not save changes: ${String(error)}`);
    }
  };

  const connectRobot = async () => {
    try {
      const [status, capabilities, remoteLibrary, remotePoseLibrary, remoteMotionLibrary] = await Promise.all([
        getStatus(connection),
        getCapabilities(connection),
        listUserScenes(connection),
        listUserPoses(connection),
        listUserMotions(connection),
      ]);
      if (
        status.api_version !== 1
        || capabilities.api_version !== 1
        || remoteLibrary.api_version !== 1
        || remotePoseLibrary.api_version !== 1
        || remoteMotionLibrary.api_version !== 1
      ) {
        throw new Error("Studio requires Orion gateway protocol version 1.");
      }
      const [remoteSources, remotePoseSources, remoteMotionSources] = await Promise.all([
        Promise.all(remoteLibrary.scenes.map((item) => getUserScene(connection, item.name))),
        Promise.all(remotePoseLibrary.assets.map((item) => getUserPose(connection, item.name))),
        Promise.all(remoteMotionLibrary.assets.map((item) => getUserMotion(connection, item.name))),
      ]);
      const remoteScenes = remoteSources.map(remoteScene);
      const remotePoses = remotePoseSources.map(remotePose);
      const remoteMotions = remoteMotionSources.map(remoteMotion);
      setSceneCatalog((current) => {
        const next = { ...current };
        for (const remote of remoteScenes) {
          if (next[remote.name]?.source !== "built_in") next[remote.name] = remote;
        }
        return next;
      });
      setPoseCatalog((current) => {
        const next = { ...current };
        for (const remote of remotePoses) {
          if (next[remote.name]?.source !== "built_in") next[remote.name] = remote;
        }
        return next;
      });
      setMotionCatalog((current) => {
        const next = { ...current };
        for (const remote of remoteMotions) {
          if (next[remote.name]?.source !== "built_in") next[remote.name] = remote;
        }
        return next;
      });
      setJointLimits(capabilities.capabilities.joint_limits);
      setRobotStatus(status);
      setRobotCapabilities(capabilities);
      setConnected(true);
      setConnectionOpen(false);
      localStorage.setItem("orionStudioGateway", gatewayUrl);
      sessionStorage.setItem("orionStudioToken", gatewayToken);
      setNotice(
        `Connected to Orion in ${status.runtime.mode} mode · loaded ${remoteScenes.length} scenes, ${remoteMotions.length} motions, and ${remotePoses.length} poses from the Pi.`,
      );
    } catch (error) {
      setConnected(false);
      setRobotCapabilities(null);
      setNotice(`Could not connect to Orion: ${String(error)}`);
    }
  };

  const runOnRobot = async () => {
    if (!connected) {
      setConnectionOpen(true);
      setNotice("Connect to Orion before running hardware capabilities.");
      return;
    }
    try {
      if (previewKind === "pose") {
        if (poseDirty) throw new Error("Save the keyframe pose before hardware playback.");
        if (poseDraft?.source === "user" && !poseDraft.remote_revision) {
          throw new Error("Save this offline-staged pose to Orion before hardware playback.");
        }
        await prepareMovement(connection);
        const event = scene.timeline.find((item) => item.type === "goto_pose");
        if (event?.type === "goto_pose") await gotoPose(connection, event.pose, event.duration_seconds);
      } else if (previewKind === "motion") {
        if (motionDirty) throw new Error("Save the motion before hardware playback.");
        if (motionDraft?.source === "user" && !motionDraft.remote_revision) {
          throw new Error("Save this offline-staged motion to Orion before hardware playback.");
        }
        await prepareMovement(connection);
        const event = scene.timeline.find((item) => item.type === "play_motion");
        if (event?.type === "play_motion") await runMotion(connection, event.motion);
      } else {
        if (dirty || scene.source === "draft") {
          throw new Error("Save As before hardware playback so Studio and Orion use the same scene document.");
        }
        if (scene.source === "user" && !scene.remote_revision) {
          const published = await publishScene(connection, sceneDocument(scene, scene.name));
          const synchronized = { ...scene, remote_revision: published.revision };
          setScene(synchronized);
          setSceneCatalog((current) => ({ ...current, [scene.name]: synchronized }));
        }
        if (scene.timeline.some((event) => event.type === "play_motion" || event.type === "goto_pose")) {
          await prepareMovement(connection);
        }
        await runScene(connection, scene.name);
      }
      setNotice(`Orion accepted the ${previewKind} capability. Following its measured result…`);
      setRobotStatus(await getStatus(connection));
    } catch (error) {
      setNotice(`Hardware run was not started: ${String(error)}`);
    }
  };

  const stopRobot = async () => {
    if (!robotStatus || !connected) return;
    try {
      if (robotStatus.scene.active) {
        await cancelRun(connection, "scene", robotStatus.scene.active.run_id);
      } else if (robotStatus.runtime.motion) {
        await cancelRun(connection, "movement", robotStatus.runtime.motion.run_id);
      }
      setRobotStatus(await getStatus(connection));
      setNotice("Cancellation was acknowledged by Orion.");
    } catch (error) {
      setNotice(`Cancellation failed: ${String(error)}`);
    }
  };

  const releaseRobotTorque = async () => {
    if (!connected) return;
    try {
      await releaseMovement(connection);
      setRobotStatus(await getStatus(connection));
      setNotice("Orion confirmed torque release.");
    } catch (error) {
      setNotice(`Torque was not released: ${String(error)}`);
    }
  };

  return (
    <div className="studio-shell">
      <header className="topbar">
        <div className="brand-lockup">
          <div className="brand-mark"><Sparkles size={18} /></div>
          <div><strong>ORION</strong><span>STUDIO</span></div>
        </div>
        <div className="project-chip"><CloudCog size={15} /><span>{projectRoot || "Pi-authoritative mode · no local checkout"}</span></div>
        <div className="topbar-actions">
          <button className="secondary-button" onClick={() => setConnectionOpen((open) => !open)}>
            {connected ? <Radio size={16} /> : <Unplug size={16} />}
            {connected ? robotStatus?.runtime.mode ?? "Connected" : "Connect robot"}
          </button>
          {previewKind === "scene" && scene.remote_revision && dirty && <button className="secondary-button" onClick={saveSceneChanges}><Save size={16} />Save changes</button>}
          <button
            className="secondary-button"
            onClick={previewKind === "pose" ? savePoseDraft : previewKind === "motion" ? saveMotionDraft : saveUserScene}
          >
            <Save size={16} />{previewKind === "pose" ? "Save pose" : previewKind === "motion" ? "Save motion" : "Save As"}
          </button>
          <button className="primary-button" onClick={runOnRobot}><Link2 size={16} />{scene.source === "user" && !scene.remote_revision && !dirty ? "Publish & Run" : "Run on Orion"}</button>
          <button className="stop-button" onClick={stopRobot} disabled={!robotStatus?.scene.active && !robotStatus?.runtime.motion}><CircleStop size={16} />Stop</button>
          {connected && robotStatus?.runtime.torque_enabled && (
            <button className="secondary-button" onClick={releaseRobotTorque} disabled={Boolean(robotStatus.scene.active || robotStatus.runtime.motion)}>
              <Power size={16} />Release torque
            </button>
          )}
        </div>
      </header>

      {connectionOpen && (
        <section className="connection-popover">
          <header><div><p className="panel-kicker">ROBOT LINK</p><h2>Connect to Orion</h2></div><span>Pi gateway v1</span></header>
          <label>Gateway URL<input value={gatewayUrl} onChange={(event) => setGatewayUrl(event.target.value)} /></label>
          <label>Studio token<input type="password" value={gatewayToken} onChange={(event) => setGatewayToken(event.target.value)} /></label>
          <button className="primary-button" onClick={connectRobot}>Connect</button>
          <small>The raw `/tmp/oriond.sock` remains private on the Pi.</small>
        </section>
      )}

      <main className="studio-workspace">
        <nav className="asset-browser" aria-label="Orion asset library">
          <div className="browser-heading"><div><p className="panel-kicker">LIBRARY</p><h2>Orion assets</h2></div><button className="icon-button" onClick={newScene} aria-label="New scene"><Plus size={17} /></button></div>

          <section className="asset-section">
            <h3>Scenes <span>{Object.keys(catalog.scenes).length}</span></h3>
            <div className="asset-list">
              {Object.values(catalog.scenes).map((item) => (
                <button key={item.name} className={`asset-item ${selectedLibraryScene === item.name ? "active" : ""}`} onClick={() => loadScene(item.name)}>
                  <Sparkles size={14} /><span>{item.name.replaceAll("_", " ")}</span>
                </button>
              ))}
            </div>
          </section>

          <section className="asset-section">
            <div className="asset-section-heading">
              <h3>Poses <span>{Object.keys(catalog.poses).length}</span></h3>
              <button className="icon-button" onClick={newPose} aria-label="New pose"><Plus size={14} /></button>
            </div>
            <div className="asset-list compact">
              {Object.keys(catalog.poses).map((name) => (
                <button key={name} className="asset-item" onClick={() => previewPose(name)}><Move3d size={14} /><span>{name.replaceAll("_", " ")}</span></button>
              ))}
            </div>
          </section>

          <section className="asset-section">
            <div className="asset-section-heading">
              <h3>Motions <span>{Object.keys(catalog.motions).length}</span></h3>
              <button className="icon-button" onClick={newMotion} aria-label="New motion"><Plus size={14} /></button>
            </div>
            <div className="asset-list compact">
              {Object.keys(catalog.motions).map((name) => (
                <button key={name} className="asset-item" onClick={() => previewMotion(name)}><Move3d size={14} /><span>{name.replaceAll("_", " ")}</span></button>
              ))}
            </div>
          </section>
        </nav>

        <section className="canvas-column">
          <div className="scene-heading">
            <div>
              <p className="panel-kicker">{previewKind.toUpperCase()} PREVIEW</p>
              <h1>{scene.name.replaceAll("_", " ")}</h1>
              <p>{scene.description}</p>
            </div>
            <div className="scene-flags">
              <span className={`source-badge ${scene.source}`}>{scene.source.replaceAll("_", " ")}</span>
              {dirty && <span className="dirty-badge">Unsaved edits</span>}
            </div>
          </div>
          <RobotViewport catalog={catalog} joints={previewJoints} light={previewLight} />
          {previewKind === "scene" && <div className="add-event-bar">
            <span>Add at {currentTime.toFixed(2)}s</span>
            <button onClick={() => addEvent("play_motion")}><Move3d size={14} />Motion</button>
            <button onClick={() => addEvent("goto_pose")}><Move3d size={14} />Pose</button>
            <button onClick={() => addEvent("light")}><Lightbulb size={14} />Light</button>
            <button onClick={() => addLightingEffect("pulse")}><Lightbulb size={14} />Pulse</button>
            <button onClick={() => addLightingEffect("breathe")}><Lightbulb size={14} />Breathe</button>
            <button onClick={() => addEvent("audio")}><Music2 size={14} />Cue</button>
          </div>}
        </section>

        <div className="inspector-column">
          {previewKind === "scene" && <section className="scene-save-card">
            <p className="panel-kicker">SAFE SAVE</p>
            <label>Save As name<input value={saveAsName} onChange={(event) => setSaveAsName(event.target.value)} /></label>
            <label>Scene description<textarea rows={3} value={scene.description} onChange={(event) => updateSceneDescription(event.target.value)} /></label>
            <p>{connected ? "Pi-authoritative" : "Offline local staging"} · <code>scenes/user/</code></p>
          </section>}
          {previewKind === "pose" && poseDraft ? (
            <PoseEditor
              pose={poseDraft}
              limits={jointLimits}
              saveAsName={poseSaveAsName}
              dirty={poseDirty}
              onSaveAsNameChange={setPoseSaveAsName}
              onDescriptionChange={(description) => {
                setPoseDraft((current) => current ? { ...current, description } : current);
                setPoseDirty(true);
              }}
              onPositionChange={updatePosePosition}
              onSave={savePoseDraft}
            />
          ) : previewKind === "motion" && motionDraft ? (
            <MotionEditor
              motion={motionDraft}
              catalog={catalog}
              saveAsName={motionSaveAsName}
              dirty={motionDirty}
              onSaveAsNameChange={setMotionSaveAsName}
              onDescriptionChange={(description) => {
                setMotionDraft((current) => current ? { ...current, description } : current);
                setMotionDirty(true);
              }}
              onKeyframesChange={updateMotionKeyframes}
              onSave={saveMotionDraft}
            />
          ) : (
            <EventInspector catalog={previewCatalog} event={selectedEvent} onChange={updateEvent} onDelete={deleteEvent} />
          )}
          <section className="robot-status-card">
            <div><Radio size={15} /><strong>{connected ? "Orion connected" : "Robot offline"}</strong></div>
            <p>{connected
              ? `Runtime ${robotStatus?.runtime.mode} · build ${robotStatus?.runtime.build_revision ?? "unknown"} · ${robotRunSummary}`
              : "Open Connect robot to pair with the Pi gateway."}</p>
            {robotCapabilities && <p>{robotCapabilities.capabilities.scene.length} scenes · {robotCapabilities.capabilities.motion.length} motions · {robotCapabilities.capabilities.goto.length} poses on Pi</p>}
          </section>
        </div>
      </main>

      <Timeline
        catalog={previewCatalog}
        scene={scene}
        duration={duration}
        currentTime={currentTime}
        playing={playing}
        selectedEventId={selectedEventId}
        onSelectEvent={setSelectedEventId}
        onMoveEvent={moveEvent}
        onSeek={(time) => { resetPreviewCues(time); setCurrentTime(time); setPlaying(false); }}
        onTogglePlay={() => {
          if (currentTime >= duration) {
            resetPreviewCues(0);
            setCurrentTime(0);
          }
          setPlaying((value) => !value);
        }}
        onRewind={() => { resetPreviewCues(0); setCurrentTime(0); setPlaying(false); }}
      />

      <footer className="statusbar">
        <span className="status-pulse" />
        <p>{notice}</p>
        <div><Volume2 size={13} />{previewCue ? `Previewing ${previewCue}` : `${catalog.cues.length} cue · ${scene.timeline.length} events`}</div>
      </footer>
    </div>
  );
}
