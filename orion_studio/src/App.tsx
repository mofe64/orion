import { attachScenePose, renameScene, sceneCatalog, previewPoses, absoluteSceneMovement } from "./lib/scenePoses";
import { sequenceMedia, withSoundDurations, moveTrackEvent, validateMediaStarts } from "./lib/trackEditing";
import { Save, CheckCircle2, AlertCircle, Activity, CircleStop, CloudUpload, Lightbulb, Link2, Mic, Move3d, Music2, Pause, Play, Plus, Power, Radio, Sparkles, SunMoon, Waypoints } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";

import { readDraft, saveDraft, discardDraft, readUserDrafts } from "./lib/drafts";
import { RunFeedback, acceptedRun, type TrackedRun } from "./components/RunFeedback";
import { AnimationLibrary } from "./components/AnimationLibrary";
import { Home } from "./components/Home";
import { EventInspector } from "./components/EventInspector";
import { MotionEditor } from "./components/MotionEditor";
import { PoseEditor } from "./components/PoseEditor";
const RobotViewport = lazy(() => import("./components/RobotViewport").then(module => ({ default: module.RobotViewport })));
import { deleteMovementComponent, moveMovementComponent } from "./lib/movementComponents";
import { Timeline, type TrackSelection } from "./components/Timeline";
import { Settings } from "./components/Settings";
import { Debug } from "./components/Debug";
import { useStudioVoice } from "./hooks/useStudioVoice";
import { loadPreferences, savePreferences } from "./lib/preferences";
import { PairingController } from "./lib/pairing";
import { PairingPanel } from "./components/PairingPanel";
import { projectCatalog } from "./lib/catalog";
import { buildSceneDocument } from "./lib/sceneDocument";
import {
  sampleCompiledTrajectory, sampleSceneLight, sampleSceneTrajectory, sceneDuration,
  sceneMarkers, triggerTime, appendSceneMotion, sequenceSceneMotions, validateSceneMotionSchedule, type SceneTrajectoryPreviews,
} from "./lib/preview";
import {
  deleteUserScene, getUserScene, updateUserScene, compileMotionPreview, gotoPose, previewScene, publishMotion,
  publishPose, publishScene, runMotion, runScene, setCharacterMode, setCharacterState,
} from "./lib/gateway";
import type {
  CompiledTrajectoryPreview, JointPositions,
  MotionDefinition, PoseDefinition, SceneDefinition, StoredMotionDocument, StoredPoseDocument,
} from "./types";

type AssetKind = "scene" | "motion" | "pose";

function clone<T>(value: T): T { return structuredClone(value); }
function displayName(value: string): string { return value.replaceAll("_", " "); }

function poseDocument(pose: PoseDefinition, name: string): StoredPoseDocument {
  return { format_version: 2, units: "radians", poses: { [name]: { description: pose.description, tags: pose.tags, idle_profile: pose.idle_profile, default_lighting: pose.default_lighting, positions: pose.positions } } };
}

function motionDocument(motion: MotionDefinition, name: string): StoredMotionDocument {
  return { format_version: 2, motion: { name, description: motion.description, space: motion.space, style: motion.style, ...(motion.space === "anchor_relative" ? { return_to_anchor: true } : {}), keyframes: motion.keyframes.map(({ hold, ...frame }) => ({ ...frame, ...(hold ? { hold } : {}) })) } };
}

function finalAbsolutePoseName(motion: MotionDefinition): string | null {
  if (motion.space !== "absolute") return null;
  return [...motion.keyframes].reverse().find((frame) => frame.pose)?.pose ?? null;
}

export default function App() {
  const initialScene = projectCatalog.scenes.acknowledge_left ?? Object.values(projectCatalog.scenes)[0];
  const initialMotion = projectCatalog.motions.look_at_left_expressive ?? Object.values(projectCatalog.motions)[0];
  const initialPose = projectCatalog.poses.attentive ?? Object.values(projectCatalog.poses)[0];
  const [trackedRun, setTrackedRun] = useState<TrackedRun | null>(null);
  const [runPending, setRunPending] = useState(false);
  const [poseEdit, setPoseEdit] = useState<{ value: PoseDefinition; eventId: string; index: number; baseScene?: SceneDefinition } | null>(null);

  const [destination, setDestination] = useState<"home" | "animation" | "create" | "settings" | "debug">("home");
  const [homeTheme, setHomeTheme] = useState<"dark" | "light">(() => { try { return localStorage.getItem("orion-studio:theme") === "light" ? "light" : "dark"; } catch { return "dark"; } });
  const [kind, setKind] = useState<AssetKind>("scene");
  const [scene, setScene] = useState(() => readDraft("scene", initialScene));
  const [motion, setMotion] = useState(() => readDraft("motion", initialMotion));
  const [pose, setPose] = useState(() => readDraft("pose", initialPose));
  const [anchorName, setAnchorName] = useState(initialPose.name);
  const [compilePhase, setCompilePhase] = useState<"static" | "compiling" | "ready" | "failed">("static");
  const [compiled, setCompiled] = useState<CompiledTrajectoryPreview | null>(null);
  const [timedScene, setTimedScene] = useState<{ scene: SceneDefinition; anchor: string; connection: unknown } | null>(null);
  const [sceneCompiled, setSceneCompiled] = useState<SceneTrajectoryPreviews>({});
  const [currentTime, setCurrentTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [selection, setSelection] = useState<TrackSelection | null>(() => scene.motion[0] ? { track: "motion", id: scene.motion[0].id } : null);
  const [saveAs, setSaveAs] = useState(`${scene.name}_studio`);
  const [publishedAssets, setPublishedAssets] = useState<Record<string, SceneDefinition | MotionDefinition | PoseDefinition>>(() => readUserDrafts());
  const currentAsset = kind === "scene" ? scene : kind === "motion" ? motion : pose;
  const originalAsset = publishedAssets[`${kind}:${currentAsset.name}`] ?? (kind === "scene" ? projectCatalog.scenes[scene.name] : kind === "motion" ? projectCatalog.motions[motion.name] : projectCatalog.poses[pose.name]);
  const [draftError, setDraftError] = useState(false);
  const assetDirty = JSON.stringify(currentAsset) !== JSON.stringify(originalAsset);
  useEffect(() => {
    if (destination !== "create") return;
    try { saveDraft(kind, currentAsset); setDraftError(false); }
    catch { setDraftError(true); setNotice("Draft could not be saved on this device. Keep Studio open until you can publish your work."); }
  }, [kind, currentAsset, destination]);
  const [notice, setNotice] = useState("Connect Orion to use voice, character and lamp controls.");
  const [connectionOpen, setConnectionOpen] = useState(false);
  const [preferences, setPreferences] = useState(loadPreferences);
  const [pairing] = useState(() => new PairingController());
  const pairingState = useSyncExternalStore(pairing.subscribe, pairing.current);
  const { connection, status, capabilities } = pairingState;
  const voice = useStudioVoice(connection, setNotice);
  const sceneTimingReady = timedScene?.scene === scene && timedScene.anchor === anchorName && timedScene.connection === connection;
  useEffect(() => {
    // Retire the previous session-only credential; secrets live in the OS store.
    sessionStorage.removeItem("orionStudioToken");
    void pairing.start();
    return () => pairing.dispose();
  }, [pairing]);
  const connectionLabel = connection ? "Orion connected"
    : pairingState.phase === "auth_required" ? "Pair Orion again"
    : ["connecting", "reconnecting", "loading"].includes(pairingState.phase) ? "Connecting to Orion…"
    : pairingState.phase === "error" ? "Connection needs attention"
    : pairingState.paired ? "Orion disconnected" : "Pair Orion";
  useEffect(() => { window.scrollTo(0,0); }, [destination]);
  const frame = useRef(0);
  useEffect(() => { try { localStorage.setItem("orion-studio:theme", homeTheme); } catch {} }, [homeTheme]);
  const editScene = (value: SceneDefinition) => {
    const draft = structuredClone(value);
    if (!draft.motion.length && draft.starting_pose) {
      const startMotion: MotionDefinition = { name: `${draft.name}_start`, description: "Move to the scene’s starting pose.", space: "absolute", style: "attentive", return_to_anchor: false, source: "draft", keyframes: [{ pose: draft.starting_pose, duration: 1.2, arrival: "settle", hold: 0 }] };
      draft.motion = [{ id: crypto.randomUUID(), at: 0, play: startMotion.name }];
      saveDraft("motion", startMotion);
      setPublishedAssets(assets => ({ ...assets, [`motion:${startMotion.name}`]: startMotion }));
    }
    setKind("scene"); setScene(draft); setSaveAs(draft.name); setAnchorName(draft.starting_pose ?? "home");
    setSelection(draft.motion[0] ? { track: "motion", id: draft.motion[0].id } : null);
    setPublishedAssets(assets => ({ ...assets, [`scene:${draft.name}`]: draft }));
    setNotice("Choose a movement to start editing.");
    setDestination("create");
  };
  const deleteScene = async (value: SceneDefinition) => {
    if (value.source === "built_in") throw new Error("Orion collection scenes cannot be deleted.");
    if (value.source === "user" || value.remote_revision) {
      if (!connection) throw new Error("Connect Orion to delete this published scene.");
      if (!capabilities?.capabilities.scene_library.delete) throw new Error("Update Orion’s Studio gateway to delete published scenes.");
      const revision = value.remote_revision ?? (await getUserScene(connection, value.name)).revision;
      await deleteUserScene(connection, value.name, revision);
    }
    discardDraft("scene", value.name);
    setPublishedAssets(assets => { const next = { ...assets }; delete next[`scene:${value.name}`]; return next; });
    if (scene.name === value.name) setScene(clone(initialScene));
    setNotice("Scene deleted.");
  };
  const navigate = (target: typeof destination) => {
    setPublishedAssets(assets => ({ ...assets, ...readUserDrafts() }));
    if (destination === "create") {
      try { saveDraft(kind, currentAsset); } catch { setNotice("Could not save this draft. Please keep the editor open."); return; }
      setPublishedAssets(assets => ({ ...assets, [`${kind}:${currentAsset.name}`]: currentAsset }));
    }
    setPlaying(false); setDestination(target);
  };

  const catalog = useMemo(() => {
    const poses = { ...projectCatalog.poses };
    const motions = { ...projectCatalog.motions };
    const scenes = { ...projectCatalog.scenes };
    for (const [key, value] of Object.entries(publishedAssets)) {
      if (key.startsWith("pose:")) poses[value.name] = value as PoseDefinition;
      if (key.startsWith("motion:")) motions[value.name] = value as MotionDefinition;
      if (key.startsWith("scene:") && projectCatalog.scenes[value.name]?.source !== "built_in") scenes[value.name] = value as SceneDefinition;
    }
    const base = { ...projectCatalog, poses, motions, scenes, jointLimits: capabilities?.capabilities.joint_limits ?? projectCatalog.jointLimits };
    return sceneCatalog(base, scene);
  }, [capabilities, publishedAssets, scene.custom_poses, scene.custom_motions]);
  const foregroundBusy = !!(status?.scene.active || status?.speech.active || (status?.runtime.motion && !status.runtime.motion.name?.startsWith("idle_")));
  const anchor = catalog.poses[anchorName] ?? initialPose;
  const compiledSceneValues = Object.values(sceneCompiled);
  const selectedMotionEvent = selection?.track === "motion"
    ? scene.motion.find((event) => event.id === selection.id)
    : undefined;
  const displayTrajectory = kind === "motion"
    ? compiled
    : selectedMotionEvent
      ? sceneCompiled[selectedMotionEvent.id] ?? null
      : compiledSceneValues[0] ?? null;
  const duration = kind === "scene" ? sceneDuration(scene, sceneCompiled) : compiled?.duration_seconds ?? 1;
  const sceneJoints = useMemo(() => sampleSceneTrajectory(scene, sceneCompiled, currentTime), [scene, sceneCompiled, currentTime]);
  const joints: JointPositions = useMemo(() => kind === "pose"
    ? pose.positions
    : kind === "scene"
      ? sceneJoints ?? anchor.positions
      : compiled
        ? sampleCompiledTrajectory(compiled, currentTime)
        : anchor.positions, [kind, pose.positions, sceneJoints, anchor.positions, compiled, currentTime]);
  const light = useMemo(() => kind === "scene" ? sampleSceneLight(scene, currentTime, sceneCompiled) : { red: 26, green: 10, blue: 1, white: 80 }, [kind, scene, currentTime, sceneCompiled]);
  const previewReady = kind === "pose"
    || (kind === "motion" && compiled !== null)
    || (kind === "scene" && (scene.motion.length === 0 || sceneTimingReady) && scene.motion.length === compiledSceneValues.length);

  useEffect(() => {
    let cancelled = false;
    setTimedScene(null); setCompiled(null); setSceneCompiled({}); setCurrentTime(0); setPlaying(false); setCompilePhase("static");
    if (destination !== "create" || !connection || kind === "pose") return () => { cancelled = true; };

    setCompilePhase("compiling");
    if (kind === "motion") {
      void compileMotionPreview(
        connection,
        motionDocument(motion, motion.name),
        anchorName,
        motion.space === "anchor_relative" ? anchorName : undefined,
      )
        .then((trajectory) => {
          if (cancelled) return;
          setCompiled(trajectory); setCompilePhase("ready");
          setNotice("Your preview is ready.");
        })
        .catch((error) => { if (!cancelled) { setCompilePhase("failed"); setNotice(error instanceof Error ? error.message : String(error)); } });
      return () => { cancelled = true; };
    }

    void (async () => {
      const trajectories: SceneTrajectoryPreviews = {};
      let startPose = anchorName;
      for (const event of [...scene.motion].sort((left, right) => left.at - right.at)) {
        if (cancelled) return;
        const definition = catalog.motions[event.play];
        if (!definition) throw new Error(`Scene references unavailable motion ${event.play}.`);
        trajectories[event.id] = await compileMotionPreview(
          connection,
          definition.source === "draft" ? motionDocument(definition, definition.name) : event.play,
          startPose,
          definition.space === "anchor_relative" ? startPose : undefined,
          Object.keys(scene.custom_poses ?? {}).length ? previewPoses(catalog) : undefined,
        );
        startPose = finalAbsolutePoseName(definition) ?? startPose;
      }
      if (cancelled) return;
      const withAudio = await withSoundDurations(scene, catalog);
      if (cancelled) return;
      const sequenced = sequenceMedia(sequenceSceneMotions(withAudio, trajectories), trajectories);
      validateSceneMotionSchedule(sequenced, trajectories);
      if (JSON.stringify(sequenced) !== JSON.stringify(scene)) { setScene(sequenced); return; }
      validateMediaStarts(sequenced,trajectories);
      setTimedScene({ scene, anchor: anchorName, connection });
      setSceneCompiled(trajectories); setCompilePhase("ready");
      const peak = Object.values(trajectories)
        .reduce((value, trajectory) => Math.max(value, trajectory.peak_velocity_rad_s), 0);
      setNotice(scene.motion.length
        ? "Your scene is ready to preview."
        : "Your light and sound preview is ready.");
    })().catch((error) => { if (!cancelled) { setCompilePhase("failed"); setNotice(error instanceof Error ? error.message : String(error)); } });
    return () => { cancelled = true; };
  }, [anchorName, catalog.motions, connection, destination, kind, motion, scene]);

  useEffect(() => {
    if (!playing) return;
    const started = performance.now() - currentTime * 1000;
    const tick = (now: number) => {
      const elapsed = (now - started) / 1000;
      if (elapsed >= duration) { setCurrentTime(duration); setPlaying(false); return; }
      setCurrentTime(elapsed); frame.current = requestAnimationFrame(tick);
    };
    frame.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame.current);
  }, [currentTime, duration, playing]);

  useEffect(() => {
    if (!playing || kind !== "scene" || !preferences.previewAudio) return;
    const started = performance.now()-currentTime*1000;
    const fired = new Set<string>();
    let active: HTMLAudioElement | null = null;
    let audioFrame = 0;
    const tick = () => {
      const elapsed = (performance.now()-started)/1000;
      for (const event of scene.audio) {
        const at = triggerTime(event,scene,sceneCompiled);
        if (at === null || at > elapsed || fired.has(event.id)) continue;
        fired.add(event.id);
        if (elapsed >= at+(event.duration ?? 0)) continue;
        active?.pause(); active = new Audio(catalog.cueUrls[event.cue]); active.currentTime = elapsed-at;
        void active.play().catch(() => setNotice("Sound could not play. Check Studio’s audio permissions."));
      }
      audioFrame = requestAnimationFrame(tick);
    };
    audioFrame = requestAnimationFrame(tick);
    return () => { cancelAnimationFrame(audioFrame); active?.pause(); };
  },[playing,scene,sceneCompiled,kind,preferences.previewAudio]);

  const chooseAsset = (nextKind: AssetKind, name: string) => {
    if (assetDirty) {
      try { saveDraft(kind, currentAsset); }
      catch { setDraftError(true); setNotice("Cannot switch assets until this draft can be saved. Publish it or discard the changes first."); return; }
    }
    setKind(nextKind); setCurrentTime(0); setPlaying(false); setSelection(null);
    if (nextKind === "scene") { const value = readDraft("scene", catalog.scenes[name]); setScene(value); setSaveAs(`${name}_studio`); setSelection(value.motion[0] ? { track: "motion", id: value.motion[0].id } : null); }
    if (nextKind === "motion") { setMotion(readDraft("motion", catalog.motions[name])); setSaveAs(`${name}_studio`); }
    if (nextKind === "pose") { setPose(readDraft("pose", catalog.poses[name])); setSaveAs(`${name}_studio`); }
  };

  const runOnOrion = async () => {
    if (!connection) { setConnectionOpen(true); setNotice("Connect to Orion before running hardware."); return; }
    if (kind === "scene" && scene.motion.length > 0 && !sceneTimingReady) {
      setNotice("Orion is still calculating the movement sequence. Wait for the preview to be ready.");
      return;
    }
    if (kind === "scene" && scene.motion.some(event => catalog.motions[event.play]?.source === "draft")) {
      setNotice("Publish your scene before playing its new movements on Orion."); return;
    }
    if (kind !== "scene" && assetDirty) {
      setNotice(`Publish this edited ${kind} under its new name before running it on Orion.`);
      return;
    }
    try {
      setRunPending(true);
      const result = kind === "scene" ? await previewScene(connection, buildSceneDocument(scene))
        : kind === "motion" ? await runMotion(connection, motion.name) : await gotoPose(connection, pose.name, 1.2);
      setTrackedRun(acceptedRun(result, kind === "scene" ? "scene" : "movement", displayName(currentAsset.name)));
      setNotice(`Orion accepted the ${kind} run.`);
    } catch (error) { setNotice(error instanceof Error ? error.message : String(error)); }
    finally { setRunPending(false); }
  };

  const saveScene = () => {
    const name = saveAs.trim().toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
    if (!name) { setNotice("Give your scene a name first."); return; }
    if (name !== scene.name && catalog.scenes[name]) { setNotice("That scene name is already in use. Choose another name."); return; }
    if (catalog.scenes[name]?.source === "built_in") { setNotice("Choose a name for your own scene."); return; }
    const saved = renameScene(scene, name);
    try {
      saveDraft("scene", saved);
      if (name !== scene.name && scene.source === "draft") discardDraft("scene", scene.name);
      setScene(saved); setSaveAs(name); setDraftError(false);
      setPublishedAssets(assets => { const next = { ...assets, [`scene:${name}`]: saved }; if (name !== scene.name && scene.source === "draft") delete next[`scene:${scene.name}`]; return next; });
      setNotice("Scene saved.");
    } catch { setDraftError(true); setNotice("Could not save your scene. Please keep the editor open."); }
  };
  const changeMovement = (eventId: string, value: MotionDefinition) => {
    if (value.owner_scene === scene.name) { setScene({ ...scene, custom_motions: { ...scene.custom_motions, [value.name]: value } }); return; }
    const definition = { ...value, name: value.source === "draft" ? value.name : `${scene.name}_part_${crypto.randomUUID().slice(0, 8)}`, source: "draft" as const };
    try { saveDraft("motion", definition); } catch { setNotice("Could not save this movement. Please try again."); return; }
    setPublishedAssets(assets => ({ ...assets, [`motion:${definition.name}`]: definition }));
    setScene({ ...scene, motion: scene.motion.map(event => event.id === eventId ? { ...event, play: definition.name } : event) });
  };

  const publish = async () => {
    const publishName = saveAs.trim().toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
    if (!publishName) { setNotice("Give your scene a name first."); return; }
    if (!connection) { setConnectionOpen(true); setNotice("Connect to Orion before publishing an asset."); return; }
    if (kind === "scene" && scene.motion.length > 0 && !sceneTimingReady) {
      setNotice("Orion is still calculating the movement sequence. Wait for the preview to be ready.");
      return;
    }
    const publishAsset = kind === "scene" ? renameScene(scene,publishName) : { ...currentAsset, name: publishName };
    try {
      if (kind === "scene") {
        if (projectCatalog.scenes[publishName]?.source === "built_in") throw new Error("Choose your own scene name; Orion collection scenes are preserved.");
        for (const event of scene.motion) {
          const dependency = catalog.motions[event.play];
          if (dependency?.source === "draft" && !dependency.owner_scene) {
            await publishMotion(connection, motionDocument(dependency, dependency.name));
            const saved = { ...dependency, source: "user" as const };
            saveDraft("motion", saved);
            setPublishedAssets(assets => ({ ...assets, [`motion:${saved.name}`]: saved }));
          }
        }
        const document = buildSceneDocument(publishAsset as SceneDefinition);
        const result = scene.source === "user" && scene.name === publishName
          ? await updateUserScene(connection,publishName,scene.remote_revision ?? (await getUserScene(connection,publishName)).revision,document)
          : await publishScene(connection,document);
        setScene({ ...publishAsset as SceneDefinition, source: "user", remote_revision: result.revision });
      }
      else if (kind === "motion") {
        await publishMotion(connection, motionDocument(motion, saveAs));
        setMotion({ ...motion, name: publishName, source: "user" });
      } else {
        await publishPose(connection, poseDocument(pose, saveAs));
        setPose({ ...pose, name: publishName, source: "user" });
      }
      setPublishedAssets(values => ({ ...values, [`${kind}:${publishName}`]: { ...publishAsset, source: "user" } }));
      setNotice(`Published ${displayName(publishName)} to Orion.`);
    } catch (error) { setNotice(error instanceof Error ? error.message : String(error)); }
  };

  const setCharacter = async (mode: "start" | "stop" | "listening" | "thinking" | "neutral") => {
    if (!connection) { setConnectionOpen(true); return; }
    try {
      if (mode === "start" || mode === "stop") await setCharacterMode(connection, mode === "start");
      else await setCharacterState(connection, mode);
      await pairing.refresh(); setNotice(`Character ${mode} accepted.`);
    } catch (error) { setNotice(error instanceof Error ? error.message : String(error)); }
  };



  const addTrackEvent = (track: TrackSelection["track"]) => {
    const id = crypto.randomUUID();
    if (track === "motion") { setScene(appendSceneMotion(scene, id, Object.keys(catalog.motions)[0],sceneCompiled)); setSelection({ track, id }); }
    if (track === "lighting") { const event = { id, at: 0, after_previous: true, effect: "constant" as const, colors: ["warm_white"], duration: 1 }; setScene(sequenceMedia({ ...scene, lighting: [...scene.lighting, event] },sceneCompiled)); setSelection({ track, id }); }
    if (track === "audio") {
      const event = { id, at: 0, after_previous: true, cue: catalog.cues[0] };
      void withSoundDurations({ ...scene,audio: [event] },catalog).then(({ audio }) => {
        setScene(previous => sequenceMedia({ ...previous,audio: [...previous.audio,audio[0]] },sceneCompiled)); setSelection({ track,id });
      }).catch(error => setNotice(error instanceof Error ? error.message : String(error)));
    }
  };
  const deleteTrackItem = (target: TrackSelection) => {
    if (target.component?.kind === "delay" && target.track !== "motion") {
      setScene(previous => ({ ...previous, [target.track]: previous[target.track].map(event => event.id === target.id ? { ...event, delay: 0 } : event) })); setSelection(null); return;
    }
    if (target.component && target.track === "motion") {
      const event = scene.motion.find(item => item.id === target.id);
      const definition = event && catalog.motions[event.play];
      if (!definition) return;
      const updated = deleteMovementComponent(definition, target.component);
      if (updated.keyframes.length) { changeMovement(target.id, updated); setSelection(null); return; }
    }
    setScene(previous => ({ ...previous, [target.track]: previous[target.track].filter(event => event.id !== target.id) }));
    setSelection(previous => previous?.id === target.id && previous.track === target.track ? null : previous);
  };
  const beginPoseEdit = (target: TrackSelection) => {
    const event = scene.motion.find(item => item.id === target.id);
    const definition = event && catalog.motions[event.play];
    if (!definition) return;
    const index = target.component?.index ?? definition.keyframes.length-1;
    const frame = definition.keyframes[index];
    let baseScene: SceneDefinition | undefined;
    let original = frame?.pose ? catalog.poses[frame.pose] : undefined;
    if (!original && event && sceneCompiled[event.id]) {
      baseScene = absoluteSceneMovement(scene,catalog,event.id,sceneCompiled[event.id]);
      const local = sceneCatalog(catalog,baseScene);
      original = local.poses[local.motions[baseScene.motion.find(item => item.id === event.id)!.play].keyframes[index].pose!];
    }
    if (!original) { setNotice("Prepare the movement preview before editing this relative pose."); return; }
    setPlaying(false); setPoseEdit({ eventId: target.id, index, value: clone(original), baseScene });
  };
  const addDelay = () => {
    if (!selection) { setNotice("Select the item that should wait, then add a delay."); return; }
    if (selection.track !== "motion") {
      setScene({ ...scene, [selection.track]: scene[selection.track].map(event => event.id === selection.id ? { ...event, delay: (event.delay ?? 0)+1 } : event) }); return;
    }
    const event = scene.motion.find(item => item.id === selection.id);
    const definition = event && catalog.motions[event.play];
    if (!event || !definition) return;
    const index = selection.component?.index ?? definition.keyframes.length-1;
    changeMovement(event.id,{ ...definition, keyframes: definition.keyframes.map((frame,i) => i === index ? { ...frame, arrival: "settle", hold: frame.hold+1 } : frame) });
    setScene(previous => ({ ...previous, motion: previous.motion.map(item => item.id === event.id ? { ...item, show_parts: true } : item) }));
    setSelection({ track: "motion", id: event.id, component: { index, kind: "delay" } });
  };
  const deleteSelection = () => { if (selection) deleteTrackItem(selection); };

  return (
    <main className={`studio-shell home-shell ${destination === "create" ? "create-shell" : ""}`} data-home-theme={homeTheme} data-reduce-motion={preferences.reduceMotion} id="main-content">
      <a className="skip-link" href="#workspace">Skip to workspace</a>
      <header className="topbar">
        <button className="brand-lockup" onClick={() => { setDestination("home"); setPlaying(false); }} aria-label="Open Orion Studio home"><span className="brand-mark" aria-hidden="true" /><span><strong>ORION</strong><small>CHARACTER STUDIO</small></span></button>
        <nav className="mode-tabs" aria-label="Studio">{(["home", "animation", "settings", ...(preferences.debugMode ? ["debug"] : [])] as const).map(item => <button key={item} aria-current={(destination === item || (item === "animation" && destination === "create")) ? "page" : undefined} className={(destination === item || (item === "animation" && destination === "create")) ? "active" : ""} onClick={() => navigate(item as typeof destination)}>{item[0].toUpperCase()+item.slice(1)}</button>)}</nav>
        <div className="topbar-actions"><button className={connection ? "connection-button connected" : "connection-button"} aria-label={connectionLabel} title={connectionLabel} aria-expanded={connectionOpen} aria-controls="orion-pairing" onClick={() => setConnectionOpen((open) => !open)}>{connection ? <Radio size={15} /> : <Link2 size={15} />}{connectionLabel}</button><button className="quiet-button home-theme-toggle" onClick={() => setHomeTheme(value => value === "dark" ? "light" : "dark")}><SunMoon size={16} />{homeTheme === "dark" ? "Light mode" : "Dark mode"}</button></div>
        {connectionOpen && <PairingPanel controller={pairing} state={pairingState} onClose={() => setConnectionOpen(false)} />}
      </header>

      {destination === "home" && <Home catalog={catalog} theme={homeTheme} voiceLabel={voice.label} listening={voice.listening} voiceAvailable={!!connection && voice.snapshot.muted !== undefined && !voice.toggling} connection={connection} status={status} onConnect={() => setConnectionOpen(true)} onVoice={() => void voice.toggle()} onCreate={() => setDestination("animation")} onDiagnostics={preferences.debugMode ? () => navigate("debug") : undefined} onRefresh={() => pairing.refresh()} onNotice={setNotice} onRun={setTrackedRun} />}
      {destination === "animation" && <AnimationLibrary previewAudio={preferences.previewAudio} catalog={catalog} theme={homeTheme} connection={connection} status={status} onEdit={editScene} onDelete={deleteScene} onRun={setTrackedRun} onNotice={setNotice} />}
      {destination === "settings" && <Settings voice={voice} preferences={preferences} onPreferences={value => { try { savePreferences(value); setPreferences(value); } catch { setNotice("Settings could not be saved on this computer."); } }} theme={homeTheme} onTheme={setHomeTheme} status={status} connected={!!connection} onConnect={() => setConnectionOpen(true)} onCharacter={async enabled => { if (!connection) return; await setCharacterMode(connection,enabled); await pairing.refresh(); }} />}
      {destination === "debug" && preferences.debugMode && <Debug onRefresh={() => pairing.refresh()} voice={voice} connection={connection} status={status} />}
      {destination === "create" && <>
        <header className="scene-editor-heading">
          <button className="quiet-button editor-back" onClick={() => navigate("animation")}>← Animation library</button>
          <div className="editor-title-row"><input className="editor-title" aria-label="Scene name" value={saveAs.replaceAll("_", " ")} onChange={event => setSaveAs(event.target.value)} placeholder="Name your scene" />
            <div className="editor-save-actions"><span role="status" className="save-indicator" aria-label={draftError ? "Could not save" : saveAs.replaceAll(" ", "_") !== scene.name ? "Name has unsaved changes" : "Saved on this device"} title={draftError ? "Could not save" : saveAs.replaceAll(" ", "_") !== scene.name ? "Name has unsaved changes" : "Saved on this device"}>{draftError ? <AlertCircle size={20} /> : saveAs.replaceAll(" ", "_") !== scene.name ? <Save size={20} /> : <CheckCircle2 size={20} />}</span><button className="secondary-button" onClick={saveScene}><Save size={16} />Save</button><button className="primary-button" disabled={!connection || (scene.motion.length > 0 && !sceneTimingReady)} onClick={() => void publish()}><CloudUpload size={16} />Publish to Orion</button></div>
          </div>
        </header>
      <section className="workspace" id="workspace">
        <section className="stage"><div className="editor-viewport"><Suspense fallback={<p>Loading model…</p>}><RobotViewport catalog={catalog} joints={poseEdit?.value.positions ?? joints} light={light} theme={homeTheme} /></Suspense></div><div className="stage-header"><span>{compilePhase === "failed" ? "Preview needs attention" : compilePhase === "compiling" ? "Preparing preview…" : previewReady ? "Ready to preview" : "Connect Orion to prepare your preview"}</span></div><div className="transport"><button className="transport-play" disabled={!previewReady} onClick={() => { if (currentTime >= duration) setCurrentTime(0); setPlaying((value) => !value); }}>{playing ? <Pause size={17} /> : <Play size={17} />}{playing ? "Pause preview" : "Preview"}</button><input aria-label="Preview time" type="range" min={0} max={duration} step={0.01} value={Math.min(currentTime, duration)} onChange={(event) => { setPlaying(false); setCurrentTime(Number(event.target.value)); }} /><output>{currentTime.toFixed(2)} / {duration.toFixed(2)} s</output><button className="primary-button" disabled={runPending || foregroundBusy || !connection || (kind === "scene" && scene.motion.length > 0 && !sceneTimingReady) || (kind !== "scene" && assetDirty)} title={!connection ? "Connect Orion first" : foregroundBusy ? "Wait for the active run or cancel it first" : assetDirty && kind !== "scene" ? "Publish your changes before running" : undefined} onClick={() => void runOnOrion()}><Radio size={15} />Play on Orion</button></div></section>
        <details className="inspector-panel" open><summary>Edit {kind === "scene" ? "selection" : kind}</summary>{poseEdit ? <><PoseEditor compact pose={poseEdit.value} limits={catalog.jointLimits} onChange={value => setPoseEdit({ ...poseEdit, value })} /><div className="pose-edit-actions"><button onClick={() => setPoseEdit(null)}>Cancel</button><button className="primary-button" onClick={() => { setScene(attachScenePose(poseEdit.baseScene ?? scene,sceneCatalog(catalog,poseEdit.baseScene ?? scene),poseEdit.eventId,poseEdit.index,poseEdit.value)); setPoseEdit(null); setNotice("Custom pose saved in this scene."); }}>Complete edit</button></div></> : kind === "scene" ? <EventInspector onChangeMovement={changeMovement} scene={scene} selection={selection} catalog={catalog} markers={[...new Set(sceneMarkers(scene, sceneCompiled).map((marker) => marker.name))]} onChange={value => setScene(sequenceMedia(value,sceneCompiled))} onDelete={deleteSelection} onEditPose={eventId => beginPoseEdit({ track: "motion", id: eventId })} /> : kind === "motion" ? <MotionEditor motion={motion} catalog={catalog} onChange={(value) => { setMotion(value); }} /> : <PoseEditor pose={pose} limits={catalog.jointLimits} onChange={(value) => { setPose(value); }} />}</details>
      </section>

      <section className="authoring-dock">{kind === "scene" ? <><div className="dock-toolbar"><div><button onClick={() => addTrackEvent("motion")}><Plus size={14} />Movement</button><button onClick={() => addTrackEvent("lighting")}><Lightbulb size={14} />Light</button><button onClick={() => addTrackEvent("audio")}><Music2 size={14} />Sound</button><button onClick={addDelay}><Plus size={14} />Delay</button></div><label>Starting pose<select value={anchorName} onChange={(event) => setAnchorName(event.target.value)}>{Object.keys(catalog.poses).filter((name) => name !== "rest").map((name) => <option key={name}>{name}</option>)}</select></label></div><Timeline onEditPose={beginPoseEdit} onMove={(target, time) => {
        setPlaying(false);
        if (target.track === "motion" && target.component) {
          const event = scene.motion.find(item => item.id === target.id);
          if (event && sceneCompiled[event.id]) changeMovement(event.id, moveMovementComponent(catalog.motions[event.play],target.component,time-event.at,sceneCompiled[event.id]));
        } else setScene(moveTrackEvent(scene, target, time, sceneCompiled));
      }} onDelete={deleteTrackItem} onToggleParts={id => setScene(previous => ({ ...previous, motion: previous.motion.map(event => event.id === id ? { ...event, show_parts: true } : event) }))} catalog={catalog} onChangeMovement={changeMovement} scene={scene} trajectories={sceneCompiled} currentTime={currentTime} selection={selection} onSelect={setSelection} onTimeChange={(time) => { setPlaying(false); setCurrentTime(time); }} /></> : <div className="asset-dock"><label>Starting pose<select value={anchorName} onChange={(event) => setAnchorName(event.target.value)}>{Object.keys(catalog.poses).filter((name) => name !== "rest").map((name) => <option key={name}>{name}</option>)}</select></label><p>{kind === "motion" ? "Select any powered pose to preview relative idle or speaking clips around that immutable anchor." : "Pose changes remain in Studio until you publish or explicitly run them."}</p></div>}</section>

      </>}

      <RunFeedback status={status} connection={connection} tracked={trackedRun} onNotice={setNotice} />
      <footer className="statusbar" aria-live="polite"><span className={connection ? "status-dot online" : "status-dot"} /><p>{notice}</p></footer>
    </main>
  );
}
