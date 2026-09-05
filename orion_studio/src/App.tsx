import { Activity, CircleStop, CloudUpload, Lightbulb, Link2, Mic, Move3d, Music2, Pause, Play, Plus, Power, Radio, Sparkles, SunMoon, Waypoints } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";

import { readDraft, saveDraft, discardDraft } from "./lib/drafts";
import { RunFeedback, acceptedRun, type TrackedRun } from "./components/RunFeedback";
import { Home } from "./components/Home";
import { EventInspector } from "./components/EventInspector";
import { MotionEditor } from "./components/MotionEditor";
import { PoseEditor } from "./components/PoseEditor";
const RobotViewport = lazy(() => import("./components/RobotViewport").then(module => ({ default: module.RobotViewport })));
import { Timeline, type TrackSelection } from "./components/Timeline";
import { VoicePanel } from "./components/VoicePanel";
import { PairingController } from "./lib/pairing";
import { PairingPanel } from "./components/PairingPanel";
import { projectCatalog } from "./lib/catalog";
import { buildSceneDocument } from "./lib/sceneDocument";
import {
  sampleCompiledTrajectory, sampleSceneLight, sampleSceneTrajectory, sceneDuration,
  sceneMarkers, validateSceneMotionSchedule, type SceneTrajectoryPreviews,
} from "./lib/preview";
import {
  compileMotionPreview, gotoPose, previewScene, publishMotion,
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
  const [destination, setDestination] = useState<"home" | "create">("home");
  const [homeTheme, setHomeTheme] = useState<"dark" | "light">("dark");
  const [kind, setKind] = useState<AssetKind>("scene");
  const [scene, setScene] = useState(() => readDraft("scene", initialScene));
  const [motion, setMotion] = useState(() => readDraft("motion", initialMotion));
  const [pose, setPose] = useState(() => readDraft("pose", initialPose));
  const [anchorName, setAnchorName] = useState(initialPose.name);
  const [compilePhase, setCompilePhase] = useState<"static" | "compiling" | "ready" | "failed">("static");
  const [compiled, setCompiled] = useState<CompiledTrajectoryPreview | null>(null);
  const [sceneCompiled, setSceneCompiled] = useState<SceneTrajectoryPreviews>({});
  const [currentTime, setCurrentTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [selection, setSelection] = useState<TrackSelection | null>(() => scene.motion[0] ? { track: "motion", id: scene.motion[0].id } : null);
  const [saveAs, setSaveAs] = useState(`${scene.name}_studio`);
  const [publishedAssets, setPublishedAssets] = useState<Record<string, SceneDefinition | MotionDefinition | PoseDefinition>>({});
  const currentAsset = kind === "scene" ? scene : kind === "motion" ? motion : pose;
  const originalAsset = publishedAssets[`${kind}:${currentAsset.name}`] ?? (kind === "scene" ? projectCatalog.scenes[scene.name] : kind === "motion" ? projectCatalog.motions[motion.name] : projectCatalog.poses[pose.name]);
  const [draftError, setDraftError] = useState(false);
  const assetDirty = JSON.stringify(currentAsset) !== JSON.stringify(originalAsset);
  useEffect(() => {
    try { saveDraft(kind, currentAsset); setDraftError(false); }
    catch { setDraftError(true); setNotice("Draft could not be saved on this device. Keep Studio open until you can publish your work."); }
  }, [kind, currentAsset]);
  const [notice, setNotice] = useState("Connect Orion to use voice, character and lamp controls.");
  const [connectionOpen, setConnectionOpen] = useState(false);
  const [voiceLabel, setVoiceLabel] = useState("Off");
  const [voiceOpen, setVoiceOpen] = useState(false);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [pairing] = useState(() => new PairingController());
  const pairingState = useSyncExternalStore(pairing.subscribe, pairing.current);
  const { connection, status, capabilities } = pairingState;
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
  const frame = useRef(0);

  const catalog = useMemo(() => {
    const poses = { ...projectCatalog.poses };
    const motions = { ...projectCatalog.motions };
    const scenes = { ...projectCatalog.scenes };
    for (const [key, value] of Object.entries(publishedAssets)) {
      if (key.startsWith("pose:")) poses[value.name] = value as PoseDefinition;
      if (key.startsWith("motion:")) motions[value.name] = value as MotionDefinition;
      if (key.startsWith("scene:")) scenes[value.name] = value as SceneDefinition;
    }
    return { ...projectCatalog, poses, motions, scenes, jointLimits: capabilities?.capabilities.joint_limits ?? projectCatalog.jointLimits };
  }, [capabilities, publishedAssets]);
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
  const peakVelocity = kind === "scene"
    ? compiledSceneValues.reduce((peak, trajectory) => Math.max(peak, trajectory.peak_velocity_rad_s), 0)
    : compiled?.peak_velocity_rad_s ?? 0;
  const previewReady = kind === "pose"
    || (kind === "motion" && compiled !== null)
    || (kind === "scene" && scene.motion.length === compiledSceneValues.length);

  useEffect(() => {
    let cancelled = false;
    setCompiled(null); setSceneCompiled({}); setCurrentTime(0); setPlaying(false); setCompilePhase("static");
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
          setNotice(`Compiled ${trajectory.motion_name} at 50 Hz · peak ${trajectory.peak_velocity_rad_s.toFixed(2)} rad/s.`);
        })
        .catch((error) => { if (!cancelled) { setCompilePhase("failed"); setNotice(error instanceof Error ? error.message : String(error)); } });
      return () => { cancelled = true; };
    }

    void (async () => {
      const trajectories: SceneTrajectoryPreviews = {};
      let startPose = anchorName;
      for (const event of [...scene.motion].sort((left, right) => left.at - right.at)) {
        const definition = catalog.motions[event.play];
        if (!definition) throw new Error(`Scene references unavailable motion ${event.play}.`);
        trajectories[event.id] = await compileMotionPreview(
          connection,
          event.play,
          startPose,
          definition.space === "anchor_relative" ? startPose : undefined,
        );
        startPose = finalAbsolutePoseName(definition) ?? startPose;
      }
      validateSceneMotionSchedule(scene, trajectories);
      if (cancelled) return;
      setSceneCompiled(trajectories); setCompilePhase("ready");
      const peak = Object.values(trajectories)
        .reduce((value, trajectory) => Math.max(value, trajectory.peak_velocity_rad_s), 0);
      setNotice(scene.motion.length
        ? `Compiled all ${scene.motion.length} scene motion clips at 50 Hz · peak ${peak.toFixed(2)} rad/s.`
        : "This scene contains lighting and sound only; its parallel tracks are ready to preview.");
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

  const publish = async () => {
    if (!connection) { setConnectionOpen(true); setNotice("Connect to Orion before publishing an asset."); return; }
    try {
      if (kind === "scene") {
        await publishScene(connection, buildSceneDocument(scene, saveAs));
        setScene({ ...scene, name: saveAs, source: "user" });
      }
      else if (kind === "motion") {
        await publishMotion(connection, motionDocument(motion, saveAs));
        setMotion({ ...motion, name: saveAs, source: "user" });
      } else {
        await publishPose(connection, poseDocument(pose, saveAs));
        setPose({ ...pose, name: saveAs, source: "user" });
      }
      setPublishedAssets(values => ({ ...values, [`${kind}:${saveAs}`]: { ...currentAsset, name: saveAs, source: "user" } }));
      setNotice(`Published ${saveAs} to Orion's v2 user catalog.`);
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
    if (track === "motion") { const event = { id, at: Math.max(0, currentTime), play: Object.keys(catalog.motions)[0] }; setScene({ ...scene, motion: [...scene.motion, event] }); setSelection({ track, id }); }
    if (track === "lighting") { const event = { id, at: Math.max(0, currentTime), effect: "settle_glow" as const, duration: 0.8 }; setScene({ ...scene, lighting: [...scene.lighting, event] }); setSelection({ track, id }); }
    if (track === "audio") { const event = { id, at: Math.max(0, currentTime), cue: catalog.cues[0] }; setScene({ ...scene, audio: [...scene.audio, event] }); setSelection({ track, id }); }
  };
  const deleteSelection = () => {
    if (!selection) return;
    setScene({ ...scene, [selection.track]: scene[selection.track].filter((event) => event.id !== selection.id) });
    setSelection(null);
  };

  return (
    <main className={`studio-shell ${destination === "home" ? "home-shell" : "create-shell"}`} data-home-theme={destination === "home" ? homeTheme : undefined} id="main-content">
      <a className="skip-link" href="#workspace">Skip to workspace</a>
      <header className="topbar">
        <button className="brand-lockup" onClick={() => { setDestination("home"); setPlaying(false); }} aria-label="Open Orion Studio home"><span className="brand-mark" aria-hidden="true" /><span><strong>ORION</strong><small>CHARACTER STUDIO</small></span></button>
        <nav className="mode-tabs" aria-label="Studio"><button aria-current={destination === "home" ? "page" : undefined} className={destination === "home" ? "active" : ""} onClick={() => { setDestination("home"); setPlaying(false); }}>Home</button><button aria-current={destination === "create" ? "page" : undefined} className={destination === "create" ? "active" : ""} onClick={() => setDestination("create")}>Create</button></nav>
        <div className="topbar-actions">{destination === "home" && <button className="quiet-button home-theme-toggle" onClick={() => setHomeTheme(value => value === "dark" ? "light" : "dark")}><SunMoon size={16} />{homeTheme === "dark" ? "Light mode" : "Dark mode"}</button>}{destination === "create" && <button className="quiet-button" aria-label="Voice" title="Voice" onClick={() => setVoiceOpen((open) => !open)}><Mic size={16} />Voice</button>}<button className={connection ? "connection-button connected" : "connection-button"} aria-label={connectionLabel} title={connectionLabel} aria-expanded={connectionOpen} aria-controls="orion-pairing" onClick={() => setConnectionOpen((open) => !open)}>{connection ? <Radio size={15} /> : <Link2 size={15} />}{connectionLabel}</button></div>
        {connectionOpen && <PairingPanel controller={pairing} state={pairingState} onClose={() => setConnectionOpen(false)} />}
        <VoicePanel onPhaseChange={setVoiceLabel} open={voiceOpen} connection={connection} onClose={() => setVoiceOpen(false)} onNotice={setNotice} />
      </header>

      {destination === "home" && <Home catalog={catalog} theme={homeTheme} voiceLabel={voiceLabel} connection={connection} status={status} onConnect={() => setConnectionOpen(true)} onVoice={() => setVoiceOpen(true)} onCreate={() => setDestination("create")} onRefresh={() => pairing.refresh()} onNotice={setNotice} onRun={setTrackedRun} />}
      {destination === "create" && <>
        <div className="mode-tabs"  aria-label="Asset editor">{(["scene", "motion", "pose"] as const).map((item) => <button aria-pressed={kind === item} className={kind === item ? "active" : ""} onClick={() => chooseAsset(item, item === "scene" ? scene.name : item === "motion" ? motion.name : pose.name)} key={item}>{item === "scene" ? <Sparkles size={15} /> : item === "motion" ? <Waypoints size={15} /> : <Move3d size={15} />}{item}</button>)}</div>
      <p className="editor-explanation">{kind === "scene" ? "Scenes combine movement, light and sound into an expression." : kind === "motion" ? "Motions describe how Orion moves between poses, including timing and expression." : "Poses describe a single body position. Motions connect them."}</p>
      <label className="compact-asset-picker">Current {kind}<select value={currentAsset.name} onChange={event => chooseAsset(kind, event.target.value)}>{Object.values(kind === "scene" ? catalog.scenes : kind === "motion" ? catalog.motions : catalog.poses).filter(asset => diagnosticsOpen || !asset.name.includes("deployment")).map(asset => <option key={asset.name} value={asset.name}>{displayName(asset.name)}</option>)}</select></label>
      <section className="character-strip" aria-label="Character controls"><div><span className={`state-orb ${status?.character.enabled ? "alive" : ""}`} /><div><strong>{status ? displayName(status.character.state) : "Character offline"}</strong><span>{status?.character.enabled ? `Anchor ready · ${status.character.active_clip ? displayName(status.character.active_clip) : "waiting calmly"}` : "Starts in character mode after a restart"}</span></div></div><div className="character-actions"><button onClick={() => void setCharacter("start")} disabled={!connection || status?.character.enabled}><Power size={14} />Start character</button>{diagnosticsOpen && <><button onClick={() => void setCharacter("listening")} disabled={!status?.character.enabled}><Radio size={14} />Listen</button><button onClick={() => void setCharacter("thinking")} disabled={!status?.character.enabled}><Activity size={14} />Think</button></>}<button onClick={() => void setCharacter("stop")} disabled={!status?.character.enabled}><CircleStop size={14} />Stop character</button><button onClick={() => setDiagnosticsOpen((open) => !open)}><Waypoints size={14} />Diagnostics</button></div></section>

      <section className="workspace" id="workspace">
        <nav className="asset-library" aria-label="Character library"><header><p className="eyebrow">Character library</p><h1>{kind === "scene" ? "Expressions" : kind === "motion" ? "Movement" : "Poses"}</h1></header><div className="asset-list">{Object.values(kind === "scene" ? catalog.scenes : kind === "motion" ? catalog.motions : catalog.poses).filter(asset => diagnosticsOpen || !asset.name.includes("deployment")).map((asset) => { const selected = asset.name === (kind === "scene" ? scene.name : kind === "motion" ? motion.name : pose.name); return <button aria-pressed={selected} className={selected ? "selected" : ""} key={asset.name} onClick={() => chooseAsset(kind, asset.name)}><span>{displayName(asset.name)}</span><small>{asset.source === "built_in" ? "Orion" : "Yours"}</small></button>; })}</div></nav>
        <section className="stage"><Suspense fallback={<p>Loading model…</p>}><RobotViewport catalog={catalog} joints={joints} light={light} /></Suspense><div className="stage-header"><div><p className="eyebrow">{displayTrajectory ? "Rust compiled preview" : kind === "scene" && scene.motion.length === 0 ? "Parallel track preview" : "Static pose"}</p><h2>{displayName(kind === "scene" ? scene.name : kind === "motion" ? motion.name : pose.name)}</h2></div><div className="stage-meta"><span>{displayTrajectory ? "Preview ready" : compilePhase === "failed" ? "Preview failed" : compilePhase === "compiling" ? "Compiling movement…" : kind === "pose" ? "Static pose" : previewReady ? "Light and sound preview" : connection ? "Static pose" : "Connect to validate movement"}</span></div></div><div className="transport"><button className="transport-play" disabled={!previewReady} onClick={() => { if (currentTime >= duration) setCurrentTime(0); setPlaying((value) => !value); }}>{playing ? <Pause size={17} /> : <Play size={17} />}{playing ? "Pause preview" : "Preview"}</button><input aria-label="Preview time" type="range" min={0} max={duration} step={0.01} value={Math.min(currentTime, duration)} onChange={(event) => { setPlaying(false); setCurrentTime(Number(event.target.value)); }} /><output>{currentTime.toFixed(2)} / {duration.toFixed(2)} s</output><button className="primary-button" disabled={runPending || foregroundBusy || !connection || (kind !== "scene" && assetDirty)} title={!connection ? "Connect Orion first" : foregroundBusy ? "Wait for the active run or cancel it first" : assetDirty && kind !== "scene" ? "Publish your changes before running" : undefined} onClick={() => void runOnOrion()}><Radio size={15} />Run on Orion</button></div></section>
        <details className="inspector-panel" open><summary>Edit {kind === "scene" ? "selected event" : kind}</summary>{kind === "scene" ? <EventInspector scene={scene} selection={selection} catalog={catalog} markers={[...new Set(sceneMarkers(scene, sceneCompiled).map((marker) => marker.name))]} onChange={setScene} onDelete={deleteSelection} /> : kind === "motion" ? <MotionEditor motion={motion} catalog={catalog} onChange={(value) => { setMotion(value); }} /> : <PoseEditor pose={pose} limits={catalog.jointLimits} onChange={(value) => { setPose(value); }} />}</details>
      </section>

      <section className="authoring-dock">{kind === "scene" ? <><div className="dock-toolbar"><div><button onClick={() => addTrackEvent("motion")}><Plus size={14} />Motion</button><button onClick={() => addTrackEvent("lighting")}><Lightbulb size={14} />Light</button><button onClick={() => addTrackEvent("audio")}><Music2 size={14} />Sound</button></div><label>Preview anchor<select value={anchorName} onChange={(event) => setAnchorName(event.target.value)}>{Object.keys(catalog.poses).filter((name) => name !== "rest").map((name) => <option key={name}>{name}</option>)}</select></label></div><Timeline scene={scene} trajectories={sceneCompiled} currentTime={currentTime} selection={selection} onSelect={setSelection} onTimeChange={(time) => { setPlaying(false); setCurrentTime(time); }} /></> : <div className="asset-dock"><label>Preview anchor<select value={anchorName} onChange={(event) => setAnchorName(event.target.value)}>{Object.keys(catalog.poses).filter((name) => name !== "rest").map((name) => <option key={name}>{name}</option>)}</select></label><p>{kind === "motion" ? "Select any powered pose to preview relative idle or speaking clips around that immutable anchor." : "Pose changes remain in Studio until you publish or explicitly run them."}</p></div>}<div className="publish-bar"><span>{draftError ? "Draft not saved" : assetDirty ? "Draft saved on this device" : "Unchanged"}</span><button disabled={!assetDirty} onClick={() => { discardDraft(kind, currentAsset.name); if (originalAsset) { if (kind === "scene") setScene(clone(originalAsset as SceneDefinition)); else if (kind === "motion") setMotion(clone(originalAsset as MotionDefinition)); else setPose(clone(originalAsset as PoseDefinition)); } }}>Discard changes</button><label>Publish as<input value={saveAs} onChange={(event) => setSaveAs(event.target.value)} /></label><button className="secondary-button" onClick={() => void publish()}><CloudUpload size={15} />Publish asset</button></div></section>

      </>}

      <RunFeedback status={status} connection={connection} tracked={trackedRun} onNotice={setNotice} />
      {diagnosticsOpen && <aside className="diagnostics-drawer"><header><div><p className="eyebrow">Live diagnostics</p><h2>Character state</h2></div><button className="icon-button" onClick={() => setDiagnosticsOpen(false)} aria-label="Close diagnostics"><CircleStop size={15} /></button></header><dl><div><dt>State</dt><dd>{status ? displayName(status.character.state) : "Unavailable"}</dd></div><div><dt>Active clip</dt><dd>{status?.character.active_clip ? displayName(status.character.active_clip) : "None"}</dd></div><div><dt>Next idle</dt><dd>{status?.character.next_idle_category ?? "Not scheduled"}</dd></div><div><dt>Anchor</dt><dd>{status?.character.active_anchor ? Object.values(status.character.active_anchor).map((value) => value.toFixed(2)).join(" · ") : "Not captured"}</dd></div><div><dt>Runtime</dt><dd>{status?.runtime.mode ?? "Offline"} · torque {status?.runtime.torque_enabled ? "on" : "off"}</dd></div><div><dt>Hardware</dt><dd>{capabilities?.capabilities.hardware_profile.variant ?? "7.4 V STS3215"} · 52 RPM ceiling</dd></div><div><dt>Calibration</dt><dd>{catalog.jointLimits.length} {capabilities ? "robot calibration ranges" : "reference ranges"}</dd></div><div><dt>Preview velocity</dt><dd>{displayTrajectory ? `${peakVelocity.toFixed(2)} rad/s peak` : "Not compiled"}</dd></div></dl></aside>}
      <footer className="statusbar" aria-live="polite"><span className={connection ? "status-dot online" : "status-dot"} /><p>{notice}</p><span>{status?.runtime.build_revision ? `runtime ${status.runtime.build_revision}` : "offline catalog"}</span></footer>
    </main>
  );
}
