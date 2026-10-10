import { attachScenePose, renameScene, sceneCatalog, previewPoses, absoluteSceneMovement } from "./lib/scenePoses";
import { sequenceMedia, withSoundDurations, moveTrackEvent, validateMediaStarts } from "./lib/trackEditing";
import { CloudUpload, Lightbulb, Link2, Music2, Pause, Play, Plus, Radio } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";

import { readDraft, saveDraft, discardDraft, readUserDrafts } from "./lib/drafts";
import { publishDisabledReason, saveNamedScene, sceneName, sceneNameProblem, sceneSaveLabel, type DraftSaveResult } from "./lib/sceneDraft";
import { RunFeedback, acceptedRun, type TrackedRun } from "./components/RunFeedback";
import { EditorFeedback } from "./components/EditorFeedback";
import { failure, type Failure } from "./lib/feedback";
import { AnimationLibrary } from "./components/AnimationLibrary";
import { Home } from "./components/Home";
import { EventInspector } from "./components/EventInspector";
import { PoseEditor } from "./components/PoseEditor";
const RobotViewport = lazy(() => import("./components/RobotViewport").then(module => ({ default: module.RobotViewport })));
import { deleteMovementComponent, moveMovementComponent } from "./lib/movementComponents";
import { Timeline, type TrackSelection } from "./components/Timeline";
import { Settings } from "./components/Settings";
import { Debug } from "./components/Debug";
import { VoiceHistory } from "./components/VoiceHistory";
import { useStudioVoice } from "./hooks/useStudioVoice";
import { useScopedFeedback } from "./hooks/useScopedNotice";
import { loadPreferences, savePreferences } from "./lib/preferences";
import { PairingController } from "./lib/pairing";
import { PairingPanel } from "./components/PairingPanel";
import { catalogForHardware, projectCatalog } from "./lib/catalog";
import { buildSceneDocument } from "./lib/sceneDocument";
import {
  sampleSceneLight, sampleSceneTrajectory, sceneDuration,
  sceneMarkers, triggerTime, appendSceneMotion, sequenceSceneMotions, validateSceneMotionSchedule, type SceneTrajectoryPreviews,
} from "./lib/preview";
import {
  deleteUserScene, getUserScene, updateUserScene, compileMotionPreview, previewScene, publishMotion,
  publishScene, setAlertSound,
} from "./lib/gateway";
import type {
  JointPositions,
  MotionDefinition, PoseDefinition, SceneDefinition, StoredMotionDocument,
} from "./types";

function clone<T>(value: T): T { return structuredClone(value); }
import { assetDisplayName as displayName } from "./lib/displayName";

function motionDocument(motion: MotionDefinition, name: string): StoredMotionDocument {
  return { format_version: 2, motion: { name, description: motion.description, space: motion.space, style: motion.style, ...(motion.space === "anchor_relative" ? { return_to_anchor: true } : {}), keyframes: motion.keyframes.map(({ hold, ...frame }) => ({ ...frame, ...(hold ? { hold } : {}) })) } };
}

function finalAbsolutePoseName(motion: MotionDefinition): string | null {
  if (motion.space !== "absolute") return null;
  return [...motion.keyframes].reverse().find((frame) => frame.pose)?.pose ?? null;
}

export default function App() {
  const initialScene = projectCatalog.scenes.acknowledge_left ?? Object.values(projectCatalog.scenes)[0];
  const initialPose = projectCatalog.poses.attentive ?? Object.values(projectCatalog.poses)[0];
  const [trackedRun, setTrackedRun] = useState<TrackedRun | null>(null);
  const [runPending, setRunPending] = useState(false);
  const [poseEdit, setPoseEdit] = useState<{ value: PoseDefinition; eventId: string; index: number; baseScene?: SceneDefinition } | null>(null);

  const [destination, setDestination] = useState<"home" | "animation" | "create" | "settings" | "debug" | "history">("home");
  const [settingsVisited, setSettingsVisited] = useState(false);
  const [homeTheme, setHomeTheme] = useState<"dark" | "light">(() => { try { return localStorage.getItem("orion-studio:theme") === "light" ? "light" : "dark"; } catch { return "dark"; } });
  const [scene, setScene] = useState(() => readDraft("scene", initialScene));
  const [anchorName, setAnchorName] = useState(initialPose.name);
  const [compilePhase, setCompilePhase] = useState<"static" | "compiling" | "ready" | "failed">("static");
  const [timedScene, setTimedScene] = useState<{ scene: SceneDefinition; anchor: string; connection: unknown } | null>(null);
  const [sceneCompiled, setSceneCompiled] = useState<SceneTrajectoryPreviews>({});
  const [currentTime, setCurrentTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [selection, setSelection] = useState<TrackSelection | null>(() => scene.motion[0] ? { track: "motion", id: scene.motion[0].id } : null);
  const [saveAs, setSaveAs] = useState(scene.name);
  const [editingSceneName, setEditingSceneName] = useState(false);
  const [nameError, setNameError] = useState(false);
  const [publishedAssets, setPublishedAssets] = useState<Record<string, SceneDefinition | MotionDefinition | PoseDefinition>>(() => readUserDrafts());
  const [pairing] = useState(() => new PairingController());
  const pairingState = useSyncExternalStore(pairing.subscribe, pairing.current);
  const { connection, status, capabilities } = pairingState;
  // The connected lamp decides which bundled model and built-in assets apply.
  const hardwareCatalog = catalogForHardware(status?.runtime.hardware);
  const [feedback, showFeedback] = useScopedFeedback<{ value: string | Failure; control: "save" | "preview" | "selection" | "timeline" | "preferences" }>(destination, connection);
  const setNotice = (message: string, control: NonNullable<typeof feedback>["control"] = "preview") => showFeedback({ value: message, control });
  const setFailure = (message: string, error: unknown, control: NonNullable<typeof feedback>["control"] = "preview") => showFeedback({ value: failure(message, error), control });
  const feedbackFor = (control: NonNullable<typeof feedback>["control"]) => feedback?.control === control ? feedback.value : null;
  const preferencesNotice = feedbackFor("preferences");
  const [draftResult, setDraftResult] = useState<DraftSaveResult | null>(null);
  const persistDraft = (value: SceneDefinition) => {
    try { saveDraft("scene", value); setDraftResult({ scene: value, failed: false }); return true; }
    catch (error) { setDraftResult({ scene: value, failed: true }); setFailure("Studio couldn't save this draft. Please keep the editor open and try again.", error, "save"); return false; }
  };
  useEffect(() => {
    if (destination === "create") persistDraft(scene);
  }, [scene, destination]);
  const draftLabel = nameError ? "Couldn't save" : sceneName(saveAs) !== scene.name ? "Saving…" : sceneSaveLabel(scene, draftResult);
  const [connectionOpen, setConnectionOpen] = useState(false);
  const [preferences, setPreferences] = useState(loadPreferences);
  const voice = useStudioVoice(connection, destination);
  const sceneTimingReady = timedScene?.scene === scene && timedScene.anchor === anchorName && timedScene.connection === connection;
  useEffect(() => {
    // Retire the previous session-only credential; secrets live in the OS store.
    sessionStorage.removeItem("orionStudioToken");
    void pairing.start();
    return () => pairing.dispose();
  }, [pairing]);
  const connectionLabel = connection ? "Orion connected"
    : pairingState.phase === "auth_required" ? "Connect Orion again"
    : ["connecting", "reconnecting", "loading"].includes(pairingState.phase) ? "Connecting to Orion…"
    : pairingState.phase === "error" ? "Connection needs attention"
    : pairingState.paired ? "Orion disconnected" : "Connect Orion";
  useEffect(() => { window.scrollTo(0,0); }, [destination]);
  const frame = useRef(0);
  const inspectorPanel = useRef<HTMLElement>(null);
  useEffect(() => { inspectorPanel.current?.scrollTo({ top: 0 }); }, [selection?.track, selection?.id, selection?.component?.kind, selection?.component?.index, !!poseEdit]);
  const editScene = (value: SceneDefinition) => {
    const draft = structuredClone(value);
    if (!draft.motion.length && draft.starting_pose) {
      const startMotion: MotionDefinition = { name: `${draft.name}_start`, description: "Move to the scene’s starting pose.", space: "absolute", style: "attentive", return_to_anchor: false, source: "draft", keyframes: [{ pose: draft.starting_pose, duration: 1.2, arrival: "settle", hold: 0 }] };
      draft.motion = [{ id: crypto.randomUUID(), at: 0, play: startMotion.name }];
      saveDraft("motion", startMotion);
      setPublishedAssets(assets => ({ ...assets, [`motion:${startMotion.name}`]: startMotion }));
    }
    setScene(draft); setNameError(false); setPoseEdit(null); setSaveAs(draft.name); setAnchorName(draft.starting_pose ?? "home");
    setSelection(draft.motion[0] ? { track: "motion", id: draft.motion[0].id } : null);
    setPublishedAssets(assets => ({ ...assets, [`scene:${draft.name}`]: draft }));
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
  };
  const navigate = (target: typeof destination) => {
    setPublishedAssets(assets => ({ ...assets, ...readUserDrafts() }));
    if (destination === "create") {
      if (!persistDraft(scene)) return;
      setPublishedAssets(assets => ({ ...assets, [`scene:${scene.name}`]: scene }));
    }
    if (target === "settings") setSettingsVisited(true);
    setPlaying(false); setDestination(target);
  };

  const catalog = useMemo(() => {
    const poses = { ...hardwareCatalog.poses };
    const motions = { ...hardwareCatalog.motions };
    const scenes = { ...hardwareCatalog.scenes };
    for (const [key, value] of Object.entries(publishedAssets)) {
      if (key.startsWith("pose:")) poses[value.name] = value as PoseDefinition;
      if (key.startsWith("motion:")) motions[value.name] = value as MotionDefinition;
      if (key.startsWith("scene:") && hardwareCatalog.scenes[value.name]?.source !== "built_in") scenes[value.name] = value as SceneDefinition;
    }
    const base = { ...hardwareCatalog, poses, motions, scenes, jointLimits: capabilities?.capabilities.joint_limits ?? hardwareCatalog.jointLimits };
    return sceneCatalog(base, scene);
  }, [hardwareCatalog, capabilities, publishedAssets, scene.custom_poses, scene.custom_motions]);
  const foregroundBusy = !!(status?.scene.active || status?.speech.active || (status?.runtime.motion && !status.runtime.motion.name?.startsWith("idle_")));
  const anchor = catalog.poses[anchorName] ?? initialPose;
  const compiledSceneValues = Object.values(sceneCompiled);
  const duration = sceneDuration(scene, sceneCompiled);
  const sceneJoints = useMemo(() => sampleSceneTrajectory(scene, sceneCompiled, currentTime), [scene, sceneCompiled, currentTime]);
  const joints: JointPositions = sceneJoints ?? anchor.positions;
  const light = useMemo(() => sampleSceneLight(scene, currentTime, sceneCompiled), [scene, currentTime, sceneCompiled]);
  const previewReady = (scene.motion.length === 0 || sceneTimingReady) && scene.motion.length === compiledSceneValues.length;
  const publishReason = publishDisabledReason(!!connection, scene.motion.length > 0 && !sceneTimingReady);

  useEffect(() => {
    let cancelled = false;
    setTimedScene(null); setSceneCompiled({}); setCurrentTime(0); setPlaying(false); setCompilePhase("static");
    if (destination !== "create" || !connection) return () => { cancelled = true; };

    setCompilePhase("compiling");
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
    })().catch((error) => { if (!cancelled) { setCompilePhase("failed"); setFailure("Studio couldn't prepare this preview. Please try again.", error); } });
    return () => { cancelled = true; };
  }, [anchorName, catalog.motions, connection, destination, scene]);

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
    if (!playing || !preferences.previewAudio) return;
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
  },[playing,scene,sceneCompiled,preferences.previewAudio]);

  const runOnOrion = async () => {
    if (!connection) { setConnectionOpen(true); setNotice("Connect to Orion before running hardware."); return; }
    if (scene.motion.length > 0 && !sceneTimingReady) {
      setNotice("Orion is still calculating the movement sequence. Wait for the preview to be ready.");
      return;
    }
    if (scene.motion.some(event => catalog.motions[event.play]?.source === "draft")) {
      setNotice("Publish your scene before playing its new movements on Orion."); return;
    }
    try {
      setRunPending(true);
      const result = await previewScene(connection, buildSceneDocument(scene));
      setTrackedRun(acceptedRun(result, "scene", displayName(scene.name)));
      setNotice("Orion accepted the scene run.");
    } catch (error) { setFailure("Studio couldn't play this on Orion. Please try again.", error); }
    finally { setRunPending(false); }
  };

  const commitSceneName = () => {
    const problem = sceneNameProblem(scene, saveAs, catalog.scenes);
    if (problem) { setNameError(true); setNotice(problem, "save"); return null; }
    const name = sceneName(saveAs);
    try {
      const saved = saveNamedScene(scene, name);
      setScene(saved); setSaveAs(name); setNameError(false); setDraftResult({ scene: saved, failed: false });
      setPublishedAssets(assets => { const next = { ...assets, [`scene:${name}`]: saved }; if (name !== scene.name && scene.source === "draft") delete next[`scene:${scene.name}`]; return next; });
      setNotice("", "save");
      return saved;
    } catch (error) { setNameError(true); setFailure("Studio couldn't save this scene name. Please keep the editor open and try again.", error, "save"); return null; }
  };
  const changeMovement = (eventId: string, value: MotionDefinition) => {
    if (value.owner_scene === scene.name) { setScene({ ...scene, custom_motions: { ...scene.custom_motions, [value.name]: value } }); return; }
    const definition = { ...value, name: value.source === "draft" ? value.name : `${scene.name}_part_${crypto.randomUUID().slice(0, 8)}`, source: "draft" as const };
    try { saveDraft("motion", definition); } catch { setNotice("Could not save this movement. Please try again.", "selection"); return; }
    setPublishedAssets(assets => ({ ...assets, [`motion:${definition.name}`]: definition }));
    setScene({ ...scene, motion: scene.motion.map(event => event.id === eventId ? { ...event, play: definition.name } : event) });
  };

  const publish = async () => {
    if (!connection) { setConnectionOpen(true); setNotice("Connect Orion to publish.", "save"); return; }
    if (scene.motion.length > 0 && !sceneTimingReady) {
      setNotice("Orion is still calculating the movement sequence. Wait for the preview to be ready.", "save"); return;
    }
    const problem = sceneNameProblem(scene, saveAs, catalog.scenes);
    if (problem) { setNameError(true); setNotice(problem, "save"); return; }
    // Publishing remains available to preserve work even if local storage fails.
    const publishAsset = renameScene(scene, sceneName(saveAs));
    try {
      for (const event of scene.motion) {
        const dependency = catalog.motions[event.play];
        if (dependency?.source === "draft" && !dependency.owner_scene) {
          await publishMotion(connection, motionDocument(dependency, dependency.name));
          const saved = { ...dependency, source: "user" as const };
          saveDraft("motion", saved);
          setPublishedAssets(assets => ({ ...assets, [`motion:${saved.name}`]: saved }));
        }
      }
      const document = buildSceneDocument(publishAsset);
      const result = scene.source === "user" && scene.name === publishAsset.name
        ? await updateUserScene(connection, publishAsset.name, scene.remote_revision ?? (await getUserScene(connection, publishAsset.name)).revision, document)
        : await publishScene(connection, document);
      const published = { ...publishAsset, source: "user" as const, remote_revision: result.revision };
      setScene(published); setSaveAs(published.name); setNameError(false);
      setPublishedAssets(values => ({ ...values, [`scene:${published.name}`]: published }));
      setNotice(`Published ${displayName(published.name)} to Orion.`, "save");
    } catch (error) { setFailure("Studio couldn't publish this to Orion. Please try again.", error, "save"); }
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
    if (!original) { setNotice("Prepare the movement preview before editing this relative pose.", "selection"); return; }
    setPlaying(false); setPoseEdit({ eventId: target.id, index, value: clone(original), baseScene });
  };
  const addDelay = () => {
    if (!selection) { setNotice("Select the item that should wait, then add a delay.", "timeline"); return; }
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
  const splitMovement = (id: string) => {
    setScene(previous => ({ ...previous, motion: previous.motion.map(event => event.id === id ? { ...event, show_parts: true } : event) }));
    setSelection({ track: "motion", id, component: { index: 0, kind: "pose" } });
  };
  const deleteSelection = () => { if (selection) deleteTrackItem(selection); };

  return (
    <main className={`studio-shell home-shell ${destination === "create" ? "create-shell" : ""}`} data-home-theme={homeTheme} data-reduce-motion={preferences.reduceMotion} id="main-content">
      <a className="skip-link" href="#workspace">Skip to workspace</a>
      <header className="topbar">
        <button className="brand-lockup" onClick={() => navigate("home")} aria-label="Open Orion Studio home"><span className="brand-mark" aria-hidden="true" /><span><strong>ORION</strong><small>CHARACTER STUDIO</small></span></button>
        <nav className="mode-tabs" aria-label="Studio">{(["home", "animation", "history", "settings", ...(preferences.debugMode ? ["debug"] : [])] as const).map(item => <button key={item} aria-current={(destination === item || (item === "animation" && destination === "create")) ? "page" : undefined} className={(destination === item || (item === "animation" && destination === "create")) ? "active" : ""} onClick={() => navigate(item as typeof destination)}>{item[0].toUpperCase()+item.slice(1)}</button>)}</nav>
        <div className="topbar-actions">{destination === "home" && !connection ? <span className="connection-button" role="status">{["unpaired", "disconnected"].includes(pairingState.phase) ? "Orion disconnected" : connectionLabel}</span> : <button className={connection ? "connection-button connected" : "connection-button"} aria-label={connectionLabel} title={connectionLabel} aria-expanded={connectionOpen} aria-controls="orion-pairing" onClick={() => setConnectionOpen((open) => !open)}>{connection ? <Radio size={15} /> : <Link2 size={15} />}{connectionLabel}</button>}</div>
        {connectionOpen && <PairingPanel controller={pairing} state={pairingState} onClose={() => setConnectionOpen(false)} />}
      </header>

      {destination === "home" && <Home catalog={catalog} theme={homeTheme} voiceLabel={voice.label} listening={voice.listening} voiceAvailable={!!connection && voice.snapshot.muted !== undefined && !voice.toggling} connection={connection} status={status} onConnect={() => setConnectionOpen(true)} onVoice={() => void voice.toggle()} onCreate={() => navigate("animation")} onDiagnostics={preferences.debugMode ? () => navigate("debug") : undefined} onRefresh={() => pairing.refresh()} voiceNotice={voice.notice} voiceError={voice.listeningError} onRun={setTrackedRun} />}
      {destination === "animation" && <AnimationLibrary previewAudio={preferences.previewAudio} catalog={catalog} theme={homeTheme} connection={connection} status={status} onEdit={editScene} onDelete={deleteScene} onRun={setTrackedRun} />}
      {(settingsVisited || destination === "settings") && <div hidden={destination !== "settings"}><Settings connectionScope={connection} notice={typeof preferencesNotice === "string" ? preferencesNotice : ""} voice={voice} preferences={preferences} onPreferences={value => { try { savePreferences(value); setPreferences(value); return true; } catch { setNotice("Settings could not be saved on this computer.", "preferences"); return false; } }} theme={homeTheme} onTheme={value => { try { localStorage.setItem("orion-studio:theme", value); setHomeTheme(value); return true; } catch { setNotice("Appearance could not be saved on this computer.", "preferences"); return false; } }} status={status} connected={!!connection} onConnect={() => setConnectionOpen(true)} onHome={() => navigate("home")} onSound={async (kind, sound) => { if (!connection) throw new Error("Connect Orion to change its sounds."); await setAlertSound(connection, kind, sound); await pairing.refresh(); }} /></div>}
      {destination === "debug" && preferences.debugMode && <Debug onRefresh={() => pairing.refresh()} onHistory={() => navigate("history")} voice={voice} connection={connection} status={status} />}
      {destination === "history" && <VoiceHistory onBack={() => navigate("home")} connected={!!connection} />}
      {destination === "create" && <>
        <header className="scene-editor-heading">
          <div className="editor-title-row">
            <button className="quiet-button editor-back" onClick={() => navigate("animation")}>← Animation library</button>
            <div className="editor-name"><label htmlFor="scene-name">Scene name</label>
              <input id="scene-name" className="editor-title" value={editingSceneName ? saveAs : displayName(saveAs)} onFocus={() => { setSaveAs(displayName(saveAs)); setEditingSceneName(true); }} onChange={event => { setSaveAs(event.target.value); setNameError(false); }} onBlur={() => { commitSceneName(); setEditingSceneName(false); }} onKeyDown={event => { if (event.key === "Enter") { event.preventDefault(); event.currentTarget.blur(); } }} placeholder="Name your scene" />
              <span role="status" className="save-indicator">{draftLabel}</span>
            </div>
            <div className="editor-save-actions">
              <button className="primary-button" disabled={!connection || (scene.motion.length > 0 && !sceneTimingReady)} title={publishReason ?? undefined} aria-describedby={publishReason ? "publish-reason" : undefined} onClick={() => void publish()}><CloudUpload size={16} />Publish to Orion</button>
              {publishReason && <p className="field-help" id="publish-reason">{publishReason}</p>}
            </div>
          </div>
          <EditorFeedback value={feedbackFor("save")} />
        </header>
        <section className="workspace" id="workspace">
          <section className="stage" aria-label="Scene preview">
            <div className="stage-header"><span>{compilePhase === "failed" ? "Preview needs attention" : compilePhase === "compiling" ? "Preparing preview…" : previewReady ? "Ready to preview" : "Connect Orion to prepare your preview"}</span></div>
            <div className="editor-viewport"><Suspense fallback={<p>Loading model…</p>}><RobotViewport catalog={catalog} joints={poseEdit?.value.positions ?? joints} light={light} theme={homeTheme} /></Suspense></div>
            <div className="transport">
              <button className="transport-play" disabled={!previewReady} onClick={() => { if (currentTime >= duration) setCurrentTime(0); setPlaying(value => !value); }}>{playing ? <Pause size={17} /> : <Play size={17} />}{playing ? "Pause preview" : "Preview"}</button>
              <input aria-label="Preview time" type="range" min={0} max={duration} step={0.01} value={Math.min(currentTime, duration)} onChange={event => { setPlaying(false); setCurrentTime(Number(event.target.value)); }} />
              <output>{currentTime.toFixed(2)} / {duration.toFixed(2)} s</output>
              <button className="primary-button" disabled={runPending || foregroundBusy || !connection || (scene.motion.length > 0 && !sceneTimingReady)} title={!connection ? "Connect Orion first" : foregroundBusy ? "Wait for the active run or cancel it first" : undefined} onClick={() => void runOnOrion()}><Radio size={15} />Play on Orion</button>
            </div>
            <EditorFeedback value={feedbackFor("preview")} />
          </section>
          <section className="inspector-panel" ref={inspectorPanel} aria-label="Selection settings">
            <EditorFeedback value={feedbackFor("selection")} />
            {poseEdit ? <>
              <PoseEditor pose={poseEdit.value} limits={catalog.jointLimits} onChange={value => setPoseEdit({ ...poseEdit, value })} />
              <div className="pose-edit-actions"><button className="quiet-button" onClick={() => setPoseEdit(null)}>Cancel</button><button className="primary-button" onClick={() => { setScene(attachScenePose(poseEdit.baseScene ?? scene, sceneCatalog(catalog, poseEdit.baseScene ?? scene), poseEdit.eventId, poseEdit.index, poseEdit.value)); setPoseEdit(null); setNotice("Custom pose saved in this scene.", "selection"); }}>Complete edit</button></div>
            </> : <EventInspector onChangeMovement={changeMovement} scene={scene} selection={selection} catalog={catalog} markers={[...new Set(sceneMarkers(scene, sceneCompiled).map(marker => marker.name))]} onChange={value => setScene(sequenceMedia(value, sceneCompiled))} onDelete={deleteSelection} onAddDelay={addDelay} onSplit={splitMovement} onEditPose={eventId => beginPoseEdit(selection?.track === "motion" && selection.id === eventId ? selection : { track: "motion", id: eventId })} />}
          </section>
        </section>
        <section className="authoring-dock">
          <div className="dock-toolbar">
            <div><button onClick={() => addTrackEvent("motion")}><Plus size={14} />Movement</button><button onClick={() => addTrackEvent("lighting")}><Lightbulb size={14} />Light</button><button onClick={() => addTrackEvent("audio")}><Music2 size={14} />Sound</button></div>
            <div className="preview-anchor"><label>Preview from<select value={anchorName} aria-describedby="preview-anchor-help" onChange={event => setAnchorName(event.target.value)}>{Object.keys(catalog.poses).filter(name => name !== "rest").map(name => <option key={name} value={name}>{displayName(name)}</option>)}</select></label><p className="field-help" id="preview-anchor-help">Orion starts from wherever it is.</p></div>
          </div>
          <EditorFeedback value={feedbackFor("timeline")} />
          <Timeline onEditPose={beginPoseEdit} onMove={(target, time) => {
            setPlaying(false);
            if (target.track === "motion" && target.component) {
              const event = scene.motion.find(item => item.id === target.id);
              if (event && sceneCompiled[event.id]) changeMovement(event.id, moveMovementComponent(catalog.motions[event.play], target.component, time - event.at, sceneCompiled[event.id]));
            } else setScene(moveTrackEvent(scene, target, time, sceneCompiled));
          }} onDelete={deleteTrackItem} onToggleParts={splitMovement} catalog={catalog} onChangeMovement={changeMovement} scene={scene} trajectories={sceneCompiled} currentTime={currentTime} selection={selection} onSelect={setSelection} onTimeChange={time => { setPlaying(false); setCurrentTime(time); }} />
        </section>
      </>}

      <RunFeedback page={destination} status={status} connection={connection} tracked={trackedRun} />
    </main>
  );
}
