import { invoke } from "@tauri-apps/api/core";
import { load as loadYaml } from "js-yaml";
import {
  Check,
  CircleStop,
  Film,
  Lightbulb,
  Link2,
  Mic,
  Move3d,
  Music2,
  Plus,
  Power,
  Radio,
  Save,
  Unplug,
  Volume2,
  Waypoints,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { EventInspector } from "./components/EventInspector";
import { MotionEditor } from "./components/MotionEditor";
import { PoseEditor } from "./components/PoseEditor";
import { RobotViewport } from "./components/RobotViewport";
import { Timeline, type TimelineSelectionMode } from "./components/Timeline";
import { VoicePanel } from "./components/VoicePanel";
import { projectCatalog } from "./lib/catalog";
import { createLightingEffect, type LightingEffectKind } from "./lib/lightingEffects";
import { buildSceneDocument } from "./lib/sceneDocument";
import { materializeSceneDraft, usedDraftPoseNames } from "./lib/sceneDraft";
import { duplicateTimelineEvents, timelineLane } from "./lib/timelineEditing";
import type { StudioVoicePhase } from "./lib/studioVoicePipeline";
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
  previewScene,
  releaseMovement,
  runMotion,
  runScene,
  updateUserScene,
  type GatewayConnection,
  type UserAssetSource,
  type UserSceneSource,
} from "./lib/gateway";
import {
  eventDuration,
  eventEndPose,
  expandSceneTimeline,
  isDelayEvent,
  sampleSceneLight,
  sampleScenePose,
  sceneDuration,
  splitSceneEvent,
} from "./lib/preview";
import type {
  GatewayCapabilities,
  GatewayStatus,
  JointName,
  MotionDefinition,
  MotionKeyframe,
  PoseDefinition,
  ProjectCatalog,
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
    throw new Error(`Robot scene '${source.name}' could not be verified.`);
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
    throw new Error(`Robot pose '${source.name}' could not be verified.`);
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
    throw new Error(`Robot motion '${source.name}' could not be verified.`);
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

function createEvent(
  type: SceneEvent["type"],
  at: number,
  catalog = projectCatalog,
  currentSceneName = "",
): SceneEvent {
  const id = crypto.randomUUID();
  switch (type) {
    case "play_motion":
      return { id, at, type, motion: Object.keys(catalog.motions)[0] };
    case "goto_pose":
      return {
        id,
        at,
        type,
        pose: catalog.poses.attentive ? "attentive" : Object.keys(catalog.poses)[0],
        duration_seconds: 2,
      };
    case "scene":
      return {
        id,
        at,
        type,
        scene: Object.keys(catalog.scenes).find((name) => name !== currentSceneName)
          ?? Object.keys(catalog.scenes)[0],
      };
    case "light":
      return { id, at, type, red: 8, green: 3, blue: 0, white: 20, transition_seconds: 0.35 };
    case "audio":
      return { id, at, type, cue: catalog.cues[0] ?? "acknowledge" };
  }
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
  const [editingTimelinePoseId, setEditingTimelinePoseId] = useState<string | null>(null);
  const [motionDraft, setMotionDraft] = useState<MotionDefinition | null>(null);
  const [motionSaveAsName, setMotionSaveAsName] = useState("my_motion");
  const [motionDirty, setMotionDirty] = useState(false);
  const [selectedLibraryScene, setSelectedLibraryScene] = useState(firstScene.name);
  const [selectedEventId, setSelectedEventId] = useState<string | null>(scene.timeline[0]?.id ?? null);
  const [selectedEventIds, setSelectedEventIds] = useState<string[]>(
    scene.timeline[0]?.id ? [scene.timeline[0].id] : [],
  );
  const [currentTime, setCurrentTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [saveAsName, setSaveAsName] = useState(`${firstScene.name}_studio`);
  const [projectRoot, setProjectRoot] = useState("");
  const [notice, setNotice] = useState("Built-in assets loaded from the Orion project.");

  const [voiceOpen, setVoiceOpen] = useState(false);
  const [voicePhase, setVoicePhase] = useState<StudioVoicePhase>("off");
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
  const reportedSceneRun = useRef<number | null>(null);
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
  const scenePreviewCatalog = useMemo(() => ({
    ...catalog,
    poses: { ...catalog.poses, ...(scene.draftPoses ?? {}) },
  }), [catalog, scene.draftPoses]);
  const previewCatalog = useMemo(() => {
    if (previewKind !== "motion" || !motionDraft) return scenePreviewCatalog;
    const event = scene.timeline.find((item) => item.type === "play_motion");
    if (event?.type !== "play_motion") return scenePreviewCatalog;
    return {
      ...scenePreviewCatalog,
      motions: { ...scenePreviewCatalog.motions, [event.motion]: motionDraft },
    };
  }, [scenePreviewCatalog, motionDraft, previewKind, scene.timeline]);
  const expandedTimeline = useMemo(
    () => expandSceneTimeline(scene, previewCatalog),
    [scene, previewCatalog],
  );

  const duration = useMemo(() => {
    const cueEnd = scene.timeline.reduce((latest, event) => {
      if (event.type !== "audio") return latest;
      return Math.max(latest, event.at + (cueDurations[event.cue] ?? 1));
    }, 0);
    const expandedCueEnd = expandedTimeline.reduce((latest, event) => {
      if (event.type !== "audio") return latest;
      return Math.max(latest, event.at + (cueDurations[event.cue] ?? 1));
    }, cueEnd);
    return Math.max(sceneDuration(scene, previewCatalog), expandedCueEnd);
  }, [scene, cueDurations, previewCatalog, expandedTimeline]);
  const sampledJoints = useMemo(
    () => sampleScenePose(scene, previewCatalog, currentTime),
    [scene, previewCatalog, currentTime],
  );
  const previewJoints = (previewKind === "pose" || editingTimelinePoseId) && poseDraft
    ? poseDraft.positions
    : sampledJoints;
  const previewLight = useMemo(
    () => sampleSceneLight(scene, currentTime, previewCatalog),
    [scene, currentTime, previewCatalog],
  );
  const selectedEvent = scene.timeline.find((event) => event.id === selectedEventId) ?? null;
  const selectOnly = (id: string | null) => {
    setSelectedEventId(id);
    setSelectedEventIds(id ? [id] : []);
  };
  const selectTimelineEvent = (id: string, mode: TimelineSelectionMode) => {
    if (mode === "preserve" && selectedEventIds.includes(id)) {
      setSelectedEventId(id);
      return;
    }
    if (mode === "toggle") {
      const clicked = scene.timeline.find((event) => event.id === id);
      const selected = scene.timeline.filter((event) => selectedEventIds.includes(event.id));
      if (!clicked || selected.some((event) => timelineLane(event) !== timelineLane(clicked))) {
        selectOnly(id);
        return;
      }
      const next = selectedEventIds.includes(id)
        ? selectedEventIds.length > 1 ? selectedEventIds.filter((item) => item !== id) : selectedEventIds
        : [...selectedEventIds, id];
      setSelectedEventIds(next);
      setSelectedEventId(next.includes(id) ? id : next.at(-1) ?? null);
      return;
    }
    if (mode === "range") {
      const anchor = scene.timeline.find((event) => event.id === selectedEventId);
      const clicked = scene.timeline.find((event) => event.id === id);
      if (!anchor || !clicked || timelineLane(anchor) !== timelineLane(clicked)) {
        selectOnly(id);
        return;
      }
      const laneEvents = scene.timeline
        .filter((event) => timelineLane(event) === timelineLane(clicked))
        .sort((left, right) => left.at - right.at);
      const anchorIndex = laneEvents.findIndex((event) => event.id === anchor.id);
      const clickedIndex = laneEvents.findIndex((event) => event.id === clicked.id);
      const [start, end] = anchorIndex < clickedIndex
        ? [anchorIndex, clickedIndex]
        : [clickedIndex, anchorIndex];
      setSelectedEventIds(laneEvents.slice(start, end + 1).map((event) => event.id));
      setSelectedEventId(id);
      return;
    }
    selectOnly(id);
  };
  const connection: GatewayConnection = { url: gatewayUrl, token: gatewayToken };
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
    for (const event of expandedTimeline) {
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
        setNotice("No offline project is configured. Connect to Orion to load and save your personal assets.");
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
    for (const event of expandedTimeline) {
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
  }, [currentTime, playing, expandedTimeline, playNextPreviewCue]);

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

  useEffect(() => {
    const result = robotStatus?.scene.last;
    if (!result || reportedSceneRun.current === result.run_id) return;
    reportedSceneRun.current = result.run_id;
    const events = result.event_count !== undefined
      ? ` · ${result.dispatched_events ?? 0}/${result.event_count} events dispatched`
      : "";
    const error = result.error ? ` · ${result.error}` : "";
    setNotice(`Orion scene “${result.name ?? result.run_id}” ${result.state}${events}${error}.`);
  }, [robotStatus?.scene.last]);

  const loadScene = (name: string) => {
    resetPreviewCues(0);
    const next = copyScene(catalog.scenes[name]);
    setScene(next);
    setPoseDraft(null);
    setEditingTimelinePoseId(null);
    setPoseDirty(false);
    setMotionDraft(null);
    setMotionDirty(false);
    setPreviewKind("scene");
    setSelectedLibraryScene(name);
    selectOnly(next.timeline[0]?.id ?? null);
    setCurrentTime(0);
    setPlaying(false);
    setDirty(false);
    setSaveAsName(`${name}_studio`);
    setNotice(next.source === "built_in"
      ? `Loaded built-in scene “${name}” as a read-only source. Save As creates a user copy.`
      : next.remote_revision
        ? `Loaded your scene “${name}” from Orion. Save changes protects newer edits; Save As creates a copy.`
        : `Loaded your offline scene “${name}”. Save As creates a separate copy.`);
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
    setEditingTimelinePoseId(null);
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
    selectOnly(next.timeline[0].id);
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
    setEditingTimelinePoseId(null);
    setPoseSaveAsName("my_keyframe");
    setPoseDirty(true);
    setMotionDraft(null);
    setMotionDirty(false);
    setScene(preview);
    setPreviewKind("pose");
    setSelectedLibraryScene("");
    selectOnly(preview.timeline[0].id);
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
    setEditingTimelinePoseId(null);
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
    selectOnly(next.timeline[0].id);
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
    setEditingTimelinePoseId(null);
    setPoseDirty(false);
    setScene(preview);
    setPreviewKind("motion");
    setSelectedLibraryScene("");
    selectOnly(preview.timeline[0].id);
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
    setEditingTimelinePoseId(null);
    setPoseDirty(false);
    setMotionDraft(null);
    setMotionDirty(false);
    setPreviewKind("scene");
    setSelectedLibraryScene("");
    selectOnly(next.timeline[0].id);
    setCurrentTime(0);
    setDirty(true);
    setSaveAsName("my_orion_scene");
    setNotice("Created a new scene. Add clips to the timeline, then choose Save As.");
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

  const deleteEvent = (eventId = selectedEventId) => {
    if (!eventId || scene.timeline.length === 1) {
      setNotice("A scene must keep at least one event.");
      return;
    }
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: current.timeline.filter((event) => event.id !== eventId),
    }));
    if (editingTimelinePoseId === eventId) {
      setEditingTimelinePoseId(null);
      setPoseDraft(null);
    }
    selectOnly(null);
    setDirty(true);
  };

  const duplicateEvents = (eventIds: string[]) => {
    if (editingTimelinePoseId) {
      setNotice("Complete the current pose edit before duplicating timeline clips.");
      return;
    }
    try {
      const duplicated = duplicateTimelineEvents(
        scene,
        eventIds,
        previewCatalog,
        () => crypto.randomUUID(),
        (event) => event.type === "audio"
          ? cueDurations[event.cue] ?? eventDuration(event, previewCatalog)
          : eventDuration(event, previewCatalog),
      );
      if (duplicated.duplicatedIds.length === 0) return;
      setScene(duplicated.scene);
      setSelectedEventIds(duplicated.duplicatedIds);
      setSelectedEventId(duplicated.duplicatedIds.at(-1) ?? null);
      setCurrentTime(duplicated.startsAt);
      setPlaying(false);
      setDirty(true);
      setNotice(
        `${duplicated.duplicatedIds.length} clip${duplicated.duplicatedIds.length === 1 ? "" : "s"} duplicated after the selected group.`,
      );
    } catch (error) {
      setNotice(String(error));
    }
  };

  const laneEnd = (type: SceneEvent["type"]): number => {
    const laneTypes = type === "play_motion" || type === "goto_pose" || type === "scene"
      ? ["play_motion", "goto_pose", "scene"]
      : type === "light"
        ? ["light"]
        : ["audio"];
    return scene.timeline.reduce((latest, event) => {
      if (!laneTypes.includes(event.type)) return latest;
      const clipDuration = event.type === "audio"
        ? cueDurations[event.cue] ?? eventDuration(event, previewCatalog)
        : eventDuration(event, previewCatalog);
      return Math.max(latest, event.at + clipDuration);
    }, 0);
  };

  const addEvent = (type: SceneEvent["type"]) => {
    const at = Number(laneEnd(type).toFixed(2));
    const event = createEvent(type, at, catalog, scene.name);
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline([...current.timeline, event]),
    }));
    selectOnly(event.id);
    setCurrentTime(at);
    setPlaying(false);
    setDirty(true);
    setNotice(`${type.replaceAll("_", " ")} appended at ${at.toFixed(2)}s.`);
  };

  const addLightingEffect = (kind: LightingEffectKind) => {
    const at = Number(laneEnd("light").toFixed(3));
    const startingLight = sampleSceneLight(scene, at, previewCatalog);
    const events = createLightingEffect(kind, at, startingLight);
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline([...current.timeline, ...events]),
    }));
    selectOnly(events[0].id);
    setCurrentTime(at);
    setPlaying(false);
    setDirty(true);
    setNotice(
      `${kind === "pulse" ? "Pulse" : "Breathe"} added as ${events.length} editable RGBW fades using the existing scene format.`,
    );
  };

  const editTimelinePose = (eventId: string) => {
    const event = scene.timeline.find((item) => item.id === eventId);
    if (event?.type !== "goto_pose") return;
    const baseline = previewCatalog.poses[event.pose];
    if (!baseline) {
      setNotice(`The baseline pose “${event.pose}” is not available.`);
      return;
    }
    setEditingTimelinePoseId(eventId);
    setPoseDraft(structuredClone(baseline));
    setPoseSaveAsName(`${baseline.name}_variation`);
    setPoseDirty(true);
    selectOnly(eventId);
    setCurrentTime(event.at + event.duration_seconds);
    setPlaying(false);
    setNotice(`Editing a scene-local copy of “${baseline.draftLabel ?? baseline.name}”. Complete edit to return to the timeline.`);
  };

  const completeTimelinePoseEdit = () => {
    if (!poseDraft || !editingTimelinePoseId) return;
    const draftId = `studio_draft_${crypto.randomUUID().replaceAll("-", "_")}`;
    setScene((current) => {
      const event = current.timeline.find((item) => item.id === editingTimelinePoseId);
      if (event?.type !== "goto_pose") return current;
      const existingDraft = current.draftPoses?.[event.pose];
      const poseName = existingDraft ? event.pose : draftId;
      const completedPose: PoseDefinition = {
        ...structuredClone(poseDraft),
        name: poseName,
        source: "draft",
        draftLabel: existingDraft?.draftLabel ?? poseDraft.draftLabel ?? poseDraft.name,
        remote_revision: undefined,
      };
      return {
        ...current,
        source: "draft",
        draftPoses: { ...current.draftPoses, [poseName]: completedPose },
        timeline: current.timeline.map((item) => (
          item.id === editingTimelinePoseId && item.type === "goto_pose"
            ? { ...item, pose: poseName }
            : item
        )),
      };
    });
    setPoseDraft(null);
    setPoseDirty(false);
    setEditingTimelinePoseId(null);
    setDirty(true);
    setNotice("Pose edit completed in memory. Studio will name and save it with the scene.");
  };

  const splitTimelineEvent = (eventId: string) => {
    const event = scene.timeline.find((item) => item.id === eventId);
    if (!event) return;
    const parts = splitSceneEvent(event, previewCatalog);
    if (parts.length === 1 && parts[0] === event) {
      setNotice("This clip has no composite parts to split.");
      return;
    }
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline(current.timeline.flatMap((item) => item.id === eventId ? parts : [item])),
    }));
    selectOnly(parts[0]?.id ?? null);
    setCurrentTime(parts[0]?.at ?? event.at);
    setDirty(true);
    const delayCount = parts.filter(isDelayEvent).length;
    setNotice(`Split “${event.type === "play_motion" ? event.motion : event.type === "scene" ? event.scene : "clip"}” into ${parts.length - delayCount} editable parts and ${delayCount} visible delays.`);
  };

  const insertDelayAfter = (eventId: string, seconds: number) => {
    if (!Number.isFinite(seconds) || seconds <= 0) return;
    const event = scene.timeline.find((item) => item.id === eventId);
    if (!event) return;
    const after = event.at + eventDuration(event, previewCatalog);
    const holdPose = eventEndPose(event, previewCatalog);
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline([
        ...current.timeline.map((item) => (
          item.id !== eventId && item.at >= after - 0.001
          ? { ...item, at: Number((item.at + seconds).toFixed(3)) }
          : item
        )),
        ...(holdPose ? [{
          id: `${eventId}:delay:${crypto.randomUUID()}`,
          at: after,
          type: "goto_pose" as const,
          pose: holdPose,
          duration_seconds: seconds,
        }] : []),
      ]),
    }));
    setDirty(true);
    setNotice(`Inserted a ${seconds.toFixed(2)}s delay after the selected clip.`);
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
      setNotice("Connect to Orion to save, or configure an offline project for local work.");
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
      setPoseCatalog((current) => ({ ...current, [saved.name]: saved }));
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
      setPoseDraft(saved);
      setPoseSaveAsName(`${saved.name}_v2`);
      setPoseDirty(false);
      setScene(preview);
      selectOnly(preview.timeline[0].id);
      setNotice(`Saved the new pose “${saved.name}”${connected ? " on Orion" : " in your offline project"}.`);
    } catch (error) {
      setNotice(`Could not save pose: ${String(error)}`);
    }
  };

  const saveMotionDraft = async () => {
    if (!motionDraft) return;
    if (!connected && !projectRoot) {
      setNotice("Connect to Orion to save, or configure an offline project for local work.");
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
      selectOnly(preview.timeline[0].id);
      setNotice(`Saved the new motion “${saved.name}”${connected ? " on Orion" : " in your offline project"}.`);
    } catch (error) {
      setNotice(`Could not save motion: ${String(error)}`);
    }
  };

  const persistSceneDraftPoses = async (poses: PoseDefinition[]): Promise<PoseDefinition[]> => {
    const saved: PoseDefinition[] = [];
    for (const pose of poses) {
      const document = poseDocument(pose, pose.name);
      const result = connected
        ? await publishPose(connection, document)
        : await invoke<{ name: string; relative_path: string }>("save_user_pose", {
          projectRoot,
          document,
        });
      saved.push({
        ...structuredClone(pose),
        name: result.name,
        source: "user",
        remote_revision: "revision" in result && typeof result.revision === "string"
          ? result.revision
          : undefined,
      });
    }
    return saved;
  };

  const saveUserScene = async () => {
    if (!connected && !projectRoot) {
      setNotice("Connect to Orion to save, or configure an offline project for local work.");
      return;
    }
    const name = saveAsName.trim();
    if (!/^[A-Za-z0-9_-]+$/.test(name)) {
      setNotice("Save As name may contain only letters, numbers, underscores, and hyphens.");
      return;
    }
    try {
      const materialized = materializeSceneDraft(scene, name, catalog);
      const savedPoses = await persistSceneDraftPoses(materialized.poses);
      const document = buildSceneDocument(materialized.scene, name, {
        ...catalog,
        poses: {
          ...catalog.poses,
          ...Object.fromEntries(savedPoses.map((pose) => [pose.name, pose])),
        },
      });
      const result = connected
        ? await publishScene(connection, document)
        : await invoke<{ name: string; relative_path: string }>("save_user_scene", {
          projectRoot,
          document,
        });
      const savedScene: SceneDefinition = {
        ...copyScene(materialized.scene),
        name: result.name,
        source: "user",
        remote_revision: "revision" in result && typeof result.revision === "string"
          ? result.revision
          : undefined,
      };
      setPoseCatalog((current) => ({
        ...current,
        ...Object.fromEntries(savedPoses.map((pose) => [pose.name, pose])),
      }));
      setScene(savedScene);
      setSceneCatalog((current) => ({ ...current, [savedScene.name]: savedScene }));
      setSelectedLibraryScene(savedScene.name);
      setDirty(false);
      setSaveAsName(`${savedScene.name}_copy`);
      setNotice(
        `Saved the new scene “${savedScene.name}”${savedPoses.length ? ` with ${savedPoses.length} scene pose${savedPoses.length === 1 ? "" : "s"}` : ""}${connected ? " on Orion" : " in your offline project"}. The original scene is unchanged.`,
      );
    } catch (error) {
      setNotice(`Could not save scene: ${String(error)}`);
    }
  };

  const saveSceneChanges = async () => {
    if (!connected || !scene.remote_revision) {
      setNotice("Connect to Orion and load one of your saved scenes before saving changes in place.");
      return;
    }
    try {
      const materialized = materializeSceneDraft(scene, scene.name, catalog);
      const savedPoses = await persistSceneDraftPoses(materialized.poses);
      const result = await updateUserScene(
        connection,
        scene.name,
        scene.remote_revision,
        buildSceneDocument(materialized.scene, scene.name, {
          ...catalog,
          poses: {
            ...catalog.poses,
            ...Object.fromEntries(savedPoses.map((pose) => [pose.name, pose])),
          },
        }),
      );
      const savedScene: SceneDefinition = {
        ...copyScene(materialized.scene),
        source: "user",
        remote_revision: result.revision,
      };
      setPoseCatalog((current) => ({
        ...current,
        ...Object.fromEntries(savedPoses.map((pose) => [pose.name, pose])),
      }));
      setScene(savedScene);
      setSceneCatalog((current) => ({ ...current, [savedScene.name]: savedScene }));
      setDirty(false);
      setNotice(`Saved changes to “${result.name}” and refreshed Orion's scene library.`);
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
      reportedSceneRun.current = status.scene.last?.run_id ?? null;
      setRobotCapabilities(capabilities);
      setConnected(true);
      setConnectionOpen(false);
      localStorage.setItem("orionStudioGateway", gatewayUrl);
      sessionStorage.setItem("orionStudioToken", gatewayToken);
      setNotice(
        `Connected to Orion · loaded ${remoteScenes.length} scenes, ${remoteMotions.length} motions, and ${remotePoses.length} poses.`,
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
      if (editingTimelinePoseId) throw new Error("Complete the current pose edit before running Orion.");
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
          const published = await publishScene(connection, buildSceneDocument(scene, scene.name, previewCatalog));
          const synchronized = { ...scene, remote_revision: published.revision };
          setScene(synchronized);
          setSceneCatalog((current) => ({ ...current, [scene.name]: synchronized }));
        }
        if (expandedTimeline.some((event) => event.type === "play_motion" || event.type === "goto_pose")) {
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

  const previewOnRobot = async () => {
    if (!connected) {
      setConnectionOpen(true);
      setNotice("Connect to Orion before previewing on the robot.");
      return;
    }
    if (!robotCapabilities?.capabilities.scene_preview) {
      setNotice("Orion needs the latest software update before hardware preview is available.");
      return;
    }
    if (editingTimelinePoseId) {
      setNotice("Complete the current pose edit before previewing on Orion.");
      return;
    }
    if (usedDraftPoseNames(scene).length > 0) {
      setNotice("Save the scene before hardware preview so Orion receives its newly created poses.");
      return;
    }
    try {
      const document = buildSceneDocument(scene, "studio_preview", previewCatalog);
      if (expandedTimeline.some((event) => event.type === "play_motion" || event.type === "goto_pose")) {
        await prepareMovement(connection);
      }
      await previewScene(connection, document);
      setRobotStatus(await getStatus(connection));
      setNotice("Orion accepted this unsaved preview. Nothing was added to its scene library.");
    } catch (error) {
      setNotice(`Hardware preview was not started: ${String(error)}`);
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
          <div><strong>ORION</strong><span>STUDIO</span></div>
        </div>
        <div className="project-chip"><span>{connected ? "Robot connected" : projectRoot ? "Local project ready" : "Connect to Orion"}</span></div>
        <div className="topbar-actions">
          <button
            className={`secondary-button ${!["off", "error"].includes(voicePhase) ? "voice-active" : ""}`}
            aria-expanded={voiceOpen}
            onClick={() => {
              setConnectionOpen(false);
              setVoiceOpen((open) => !open);
            }}
          >
            <Mic size={16} />
            {voicePhase === "ready"
              ? "Voice ready"
              : voicePhase === "command_listening"
                ? "Listening…"
                : voicePhase === "transcribing"
                  ? "Transcribing…"
                  : voicePhase === "thinking"
                    ? "Thinking…"
                    : voicePhase === "synthesizing"
                      ? "Preparing voice…"
                      : voicePhase === "speaking"
                        ? "Speaking…"
                        : "Voice"}
          </button>
          <button className="secondary-button" onClick={() => {
            setVoiceOpen(false);
            setConnectionOpen((open) => !open);
          }}>
            {connected ? <Radio size={16} /> : <Unplug size={16} />}
            {connected ? robotStatus?.runtime.mode ?? "Connected" : "Connect robot"}
          </button>
          {previewKind === "scene" && scene.remote_revision && dirty && !editingTimelinePoseId && <button className="secondary-button" onClick={saveSceneChanges}><Save size={16} />Save changes</button>}
          <button
            className="secondary-button"
            onClick={editingTimelinePoseId ? completeTimelinePoseEdit : previewKind === "pose" ? savePoseDraft : previewKind === "motion" ? saveMotionDraft : saveUserScene}
          >
            {editingTimelinePoseId ? <Check size={16} /> : <Save size={16} />}
            {editingTimelinePoseId ? "Complete edit" : previewKind === "pose" ? "Save pose" : previewKind === "motion" ? "Save motion" : "Save As"}
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

      <VoicePanel
        open={voiceOpen}
        onClose={() => setVoiceOpen(false)}
        onNotice={setNotice}
        onPhaseChange={setVoicePhase}
      />

      {connectionOpen && (
        <section className="connection-popover">
          <header><div><p className="panel-kicker">ROBOT LINK</p><h2>Connect to Orion</h2></div><span>Secure connection</span></header>
          <label>Robot address<input value={gatewayUrl} onChange={(event) => setGatewayUrl(event.target.value)} /></label>
          <label>Pairing token<input type="password" value={gatewayToken} onChange={(event) => setGatewayToken(event.target.value)} /></label>
          <button className="primary-button" onClick={connectRobot}>Connect</button>
          <small>Studio sends only named poses, motions, scenes, and playback commands.</small>
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
                  <Film size={14} /><span>{item.name.replaceAll("_", " ")}</span>
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
                <button key={name} className="asset-item" onClick={() => previewMotion(name)}><Waypoints size={14} /><span>{name.replaceAll("_", " ")}</span></button>
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
            <span>New clips snap to the end of their track</span>
            <button onClick={() => addEvent("scene")}><Film size={14} />Scene</button>
            <button onClick={() => addEvent("play_motion")}><Waypoints size={14} />Motion</button>
            <button onClick={() => addEvent("goto_pose")}><Move3d size={14} />Pose</button>
            <button onClick={() => addEvent("light")}><Lightbulb size={14} />Light</button>
            <button onClick={() => addLightingEffect("pulse")}><Lightbulb size={14} />Pulse</button>
            <button onClick={() => addLightingEffect("breathe")}><Lightbulb size={14} />Breathe</button>
            <button onClick={() => addEvent("audio")}><Music2 size={14} />Cue</button>
          </div>}
        </section>

        <div className="inspector-column">
          {previewKind === "scene" && <section className="scene-save-card">
            <p className="panel-kicker">{scene.remote_revision ? "EDIT USER SCENE" : "SAFE SAVE"}</p>
            <label>{scene.remote_revision ? "Duplicate as name" : "Save As name"}<input value={saveAsName} onChange={(event) => setSaveAsName(event.target.value)} /></label>
            <label>Scene description<textarea rows={3} value={scene.description} onChange={(event) => updateSceneDescription(event.target.value)} /></label>
            {scene.remote_revision ? (
              <>
                <button className="primary-button" disabled={!dirty || !connected} onClick={saveSceneChanges}>
                  <Save size={15} />Save changes
                </button>
                <p>Edit this scene directly, or use Save As to create a separate copy.</p>
              </>
            ) : (
              <p>Saved as a personal scene. Orion's built-in scenes remain unchanged.</p>
            )}
          </section>}
          {(editingTimelinePoseId || previewKind === "pose") && poseDraft ? (
            <PoseEditor
              pose={poseDraft}
              limits={jointLimits}
              saveAsName={poseSaveAsName}
              dirty={poseDirty}
              sceneDraft={Boolean(editingTimelinePoseId)}
              onSaveAsNameChange={setPoseSaveAsName}
              onDescriptionChange={(description) => {
                setPoseDraft((current) => current ? { ...current, description } : current);
                setPoseDirty(true);
              }}
              onPositionChange={updatePosePosition}
              onSave={editingTimelinePoseId ? completeTimelinePoseEdit : savePoseDraft}
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
            <EventInspector catalog={previewCatalog} currentSceneName={scene.name} event={selectedEvent} onChange={updateEvent} onDelete={deleteEvent} />
          )}
        </div>
      </main>

      <Timeline
        catalog={previewCatalog}
        scene={scene}
        duration={duration}
        currentTime={currentTime}
        playing={playing}
        selectedEventIds={selectedEventIds}
        hardwarePreviewEnabled={connected && !Boolean(robotStatus?.scene.active || robotStatus?.runtime.motion)}
        onSelectEvent={selectTimelineEvent}
        onMoveEvent={moveEvent}
        onDeleteEvent={deleteEvent}
        onDuplicateEvents={duplicateEvents}
        onEditPose={editTimelinePose}
        onSplitEvent={splitTimelineEvent}
        onInsertDelay={insertDelayAfter}
        onSeek={(time) => { resetPreviewCues(time); setCurrentTime(time); setPlaying(false); }}
        onTogglePlay={() => {
          if (currentTime >= duration) {
            resetPreviewCues(0);
            setCurrentTime(0);
          }
          setPlaying((value) => !value);
        }}
        onPreviewHardware={previewOnRobot}
        onRewind={() => { resetPreviewCues(0); setCurrentTime(0); setPlaying(false); }}
      />

      <footer className="statusbar">
        <span className="status-pulse" />
        <p>{notice}</p>
        <div><Volume2 size={13} />{robotStatus?.scene.active?.event_count !== undefined
          ? `Orion ${robotStatus.scene.active.dispatched_events ?? 0}/${robotStatus.scene.active.event_count} events`
          : previewCue
            ? `Previewing ${previewCue}`
            : `${catalog.cues.length} cue · ${scene.timeline.length} events`}</div>
      </footer>
    </div>
  );
}
