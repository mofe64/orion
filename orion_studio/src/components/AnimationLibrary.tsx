import { sceneCatalog } from "../lib/scenePoses";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, House, Play, Plus, Radio, Square, Trash2, X } from "lucide-react";
import type { GatewayStatus, ProjectCatalog, SceneDefinition } from "../types";
import { gotoPose, previewScene, runScene, type GatewayConnection } from "../lib/gateway";
import { createSceneDraft, label, movementOnly, prepareAnimation } from "../lib/animation";
import { buildSceneDocument } from "../lib/sceneDocument";
import { EFFECT_PREVIEWS, sampleSceneLight, sampleSceneTrajectory, sceneDuration, triggerTime } from "../lib/preview";
import { acceptedRun, type TrackedRun } from "./RunFeedback";
import "./AnimationLibrary.css";
const RobotViewport = lazy(() => import("./RobotViewport").then(module => ({ default: module.RobotViewport })));
type Prepared = Awaited<ReturnType<typeof prepareAnimation>>;
interface Props {
  previewAudio?: boolean;
  catalog: ProjectCatalog; theme: "dark" | "light"; connection: GatewayConnection | null;
  status: GatewayStatus | null; onEdit: (scene: SceneDefinition) => void;
  onDelete?: (scene: SceneDefinition) => Promise<void>;
  onRun: (run: TrackedRun) => void; onNotice: (message: string) => void;
}
export function AnimationLibrary({ catalog: baseCatalog, previewAudio = true, theme, connection, status, onEdit, onDelete, onRun, onNotice }: Props) {
  const [kind, setKind] = useState<"scene" | "pose">("scene");
  const [selected, setSelected] = useState("acknowledge_left");
  const [query, setQuery] = useState("");
  const [full, setFull] = useState(true);
  const [heldPose, setHeldPose] = useState("home");
  const [prepared, setPrepared] = useState<Prepared | null>(null);
  const [activePreview, setActivePreview] = useState<Prepared | null>(null);
  const [playing, setPlaying] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [message, setMessage] = useState("");
  const [pending, setPending] = useState(false);
  const [creator, setCreator] = useState(false);
  const [seedKind, setSeedKind] = useState<"scene" | "pose">("scene");
  const [seed, setSeed] = useState("acknowledge_left");
  const [deleteError, setDeleteError] = useState("");
  const [deleting, setDeleting] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const deleteDialog = useRef<HTMLDialogElement>(null);
  useEffect(() => { if (deleteOpen) deleteDialog.current?.showModal(); }, [deleteOpen]);
  const dialog = useRef<HTMLDialogElement>(null);
  const audio = useRef<HTMLAudioElement[]>([]);
  const generation = useRef(0);
  const operation = useRef(false);
  const selectedScene = kind === "scene" ? baseCatalog.scenes[selected] : null;
  const catalog = useMemo(() => selectedScene ? sceneCatalog(baseCatalog,selectedScene) : baseCatalog,[baseCatalog,selectedScene]);
  const items = kind === "scene" ? catalog.scenes : Object.fromEntries(Object.entries(catalog.poses).filter(([,pose]) => !pose.owner_scene));
  const asset = items[selected];
  const busy = !!(status?.scene.active || status?.speech.active || (status?.runtime.motion && !status.runtime.motion.name?.startsWith("idle_")));
  const robotHome = status?.character.state === "home_idle" && !busy;
  const stopAudio = () => { audio.current.forEach(sound => { sound.pause(); sound.currentTime = 0; }); audio.current = []; };
  useEffect(() => () => { generation.current++; stopAudio(); }, []);
  useEffect(() => {
    const version = ++generation.current;
    setPrepared(null); setPlaying(false); setElapsed(0); stopAudio();
    if (!selectedScene) { setMessage(selected === heldPose ? `Showing ${label(heldPose)} in the preview.` : "Select Preview pose to see this position."); return; }
    if (!connection) { setMessage("Connect Orion to prepare scene playback. You can still explore poses."); return; }
    setMessage("Preparing scene…");
    void prepareAnimation(connection, selectedScene, catalog, heldPose).then(result => {
      if (generation.current !== version) return;
      setPrepared(result); setMessage("Ready to preview");
    }).catch(error => { if (generation.current === version) setMessage(String(error instanceof Error ? error.message : error)); });
    return () => { generation.current++; };
  }, [selectedScene, connection, catalog, heldPose]);
  useEffect(() => {
    if (!creator) return;
    dialog.current?.showModal();
    dialog.current?.querySelector("select")?.focus();
  }, [creator]);
  useEffect(() => {
    if (!playing || !activePreview) return;
    const active = full ? activePreview.scene : movementOnly(activePreview.scene);
    const duration = sceneDuration(active, activePreview.trajectories);
    const start = performance.now();
    const fired = new Set<string>();
    let frame = 0;
    const tick = (now: number) => {
      const time = Math.min(duration, (now - start) / 1000);
      setElapsed(time);
      for (const event of previewAudio ? active.audio : []) {
        const at = triggerTime(event, active, activePreview.trajectories);
        const url = catalog.cueUrls[event.cue];
        if (at !== null && at <= time && url && !fired.has(event.id)) {
          fired.add(event.id);
          const sound = new Audio(url); audio.current.push(sound);
          void sound.play().catch(() => setMessage("Sound could not play. Check Studio’s audio permissions."));
        }
      }
      if (time >= duration) {
        setPlaying(false); setHeldPose(activePreview.finalPose);
      } else frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => { cancelAnimationFrame(frame); stopAudio(); };
  }, [playing, activePreview, full, catalog, previewAudio]);
  const activeScene = activePreview && (full ? activePreview.scene : movementOnly(activePreview.scene));
  const joints = playing && activePreview ? sampleSceneTrajectory(activePreview.scene, activePreview.trajectories, elapsed) ?? (catalog.poses[heldPose] ?? catalog.poses.home).positions : (catalog.poses[heldPose] ?? catalog.poses.home).positions;
  const light = playing && activeScene && activePreview && (full)
    ? sampleSceneLight(activeScene, elapsed, activePreview.trajectories)
    : EFFECT_PREVIEWS[(catalog.poses[heldPose] ?? catalog.poses.home).default_lighting ?? "warm_idle_breathe"];
  const run = async (home = false) => {
    if (!connection || operation.current || busy || (home && (!status || robotHome)) || (!home && selectedScene?.motion.some(event => catalog.motions[event.play]?.source === "draft"))) return;
    operation.current = true; setPending(true);
    try {
      const result = home ? await runScene(connection, "return_home") : kind === "pose"
        ? await gotoPose(connection, selected, 1.2)
        : prepared ? await previewScene(connection, buildSceneDocument(full ? prepared.scene : movementOnly(prepared.scene))) : null;
      if (result) { const tracked = acceptedRun(result, home || kind === "scene" ? "scene" : "movement", home ? "Return to home pose" : label(selected)); if (tracked) onRun(tracked); }
    } catch (error) { onNotice(error instanceof Error ? error.message : String(error)); }
    finally { operation.current = false; setPending(false); }
  };
  const select = (name: string) => { setPlaying(false); stopAudio(); setSelected(name); setElapsed(0); };
  const changeKind = (next: "scene" | "pose") => { setKind(next); select(next === "scene" ? "acknowledge_left" : "home"); };
  return <section className="animation-page" id="workspace">
    <header className="animation-heading"><div><p className="eyebrow">ORION STUDIO</p><h1>Animation</h1><p>Explore a little personality.</p></div><button className="primary-button" onClick={() => { setSeedKind(kind); setSeed(selected); setCreator(true); }}><Plus size={17} />Create scene</button></header>
    <div className="animation-layout">
      <nav className="animation-library" aria-label="Animation library">
        <div className="animation-tabs">{(["scene", "pose"] as const).map(value => <button key={value} aria-pressed={kind === value} onClick={() => changeKind(value)}>{value === "scene" ? "Scenes" : "Poses"}</button>)}</div>
        <input aria-label="Search animations" placeholder={`Find a ${kind}…`} value={query} onChange={event => setQuery(event.target.value)} />
        {(["built_in", "user"] as const).map(group => <section key={group}><h2>{group === "built_in" ? "Orion collection" : kind === "scene" ? "My scenes" : "My poses"}</h2>
          {Object.values(items).filter(item => (item.source === "built_in") === (group === "built_in") && !item.name.includes("deployment") && label(item.name).toLowerCase().includes(query.toLowerCase())).map(item => <button className="animation-item" key={item.name} aria-pressed={selected === item.name} onClick={() => select(item.name)}><span>{label(item.name)}</span>{item.source === "draft" && <small>Draft</small>}</button>)}
          {group === "user" && !Object.values(items).some(item => item.source !== "built_in") && <p className="animation-empty">Your creations will live here.</p>}
        </section>)}
      </nav>
      <section className="animation-preview" aria-label="Animation preview">
        <header><div><p className="eyebrow">{asset?.source === "built_in" ? "Orion collection" : "Your creation"}</p><h2>{label(selected)}</h2><p>{asset?.description}</p></div>{kind === "scene" && asset && asset.source !== "built_in" && <div className="animation-scene-actions"><button className="quiet-button" onClick={() => onEdit(catalog.scenes[selected])}>Edit scene</button>{onDelete && <button className="danger-button" onClick={() => { setDeleteError(""); setDeleteOpen(true); }}><Trash2 size={15} />Delete scene</button>}</div>}</header>
        <div className="animation-model"><Suspense fallback={<p>Loading Orion…</p>}><RobotViewport catalog={catalog} joints={joints} light={light} mode="home" theme={theme} /></Suspense></div>
        <div className="animation-controls">
          {kind === "scene" && <div className="animation-tabs" aria-label="Scene playback content"><button disabled={playing} aria-pressed={full} onClick={() => setFull(true)}>Full scene</button><button disabled={playing} aria-pressed={!full} onClick={() => setFull(false)}>Movement only</button></div>}
          <div className="animation-play"><button className="primary-button" disabled={kind === "scene" && (!prepared || pending)} onClick={() => { if (kind === "pose") setHeldPose(selected); else { setActivePreview(prepared); setElapsed(0); setPlaying(value => !value); } }}>{playing ? <Square size={16} /> : <Play size={16} />}{kind === "pose" ? "Preview pose" : playing ? "Stop preview" : "Play preview"}</button><button className="secondary-button" disabled={!connection || pending || busy || (kind === "scene" && (!prepared || !!selectedScene?.motion.some(event => catalog.motions[event.play]?.source === "draft")))} onClick={() => void run()}><Radio size={16} />{kind === "pose" ? "Go to pose on Orion" : "Play on Orion"}</button></div>
          <p role="status">{playing ? `Playing preview · ${elapsed.toFixed(1)} s` : message}</p>
          <div className="animation-home"><button className="quiet-button" disabled={!connection || !status || robotHome || pending || busy} title={robotHome ? "Orion is already at home" : !connection ? "Connect Orion first" : busy ? "Wait for the current movement to finish" : "Play Orion’s return-home scene"} onClick={() => void run(true)}><House size={18} />Return to home pose</button>{robotHome && <span>Orion is at home</span>}</div>
        </div>
      </section>
    </div>
    {deleteOpen && selectedScene && <dialog className="scene-dialog" ref={deleteDialog} aria-labelledby="delete-scene-title" onCancel={event => { if (deleting) event.preventDefault(); else setDeleteOpen(false); }}>
      <h2 id="delete-scene-title">Delete {label(selected)}?</h2>
      <p>{selectedScene.source === "user" || selectedScene.remote_revision ? "This removes the scene from Orion and this device." : "This removes the scene saved on this device."} Scene-specific poses will be deleted too. Library poses and sounds will remain available.</p>
      {deleteError && <p role="alert">{deleteError}</p>}
      <footer><button className="quiet-button" disabled={deleting} onClick={() => setDeleteOpen(false)}>Cancel</button><button className="danger-button" disabled={deleting} onClick={() => { if (!onDelete) return; setDeleting(true); void onDelete(selectedScene).then(() => { setDeleteOpen(false); select("acknowledge_left"); }).catch(error => { setDeleteError(error instanceof Error ? error.message : String(error)); }).finally(() => setDeleting(false)); }}>{deleting ? "Deleting…" : "Delete scene"}</button></footer>
    </dialog>}
    {creator && <dialog className="scene-dialog" ref={dialog} onCancel={() => setCreator(false)} onClose={() => setCreator(false)} aria-labelledby="new-scene-title"><header><div><p className="eyebrow">MAKE IT YOURS</p><h2 id="new-scene-title">Every scene starts somewhere.</h2></div><button className="icon-button" aria-label="Close create scene" onClick={() => setCreator(false)}><X /></button></header><p>Choose a starting point. Your scene is a separate copy.</p><div className="animation-tabs"><button aria-pressed={seedKind === "scene"} onClick={() => { setSeedKind("scene"); setSeed("acknowledge_left"); }}>Build from a scene</button><button aria-pressed={seedKind === "pose"} onClick={() => { setSeedKind("pose"); setSeed("home"); }}>Start from a pose</button></div><label>Starting {seedKind}<select autoFocus value={seed} onChange={event => setSeed(event.target.value)}>{Object.keys(seedKind === "scene" ? catalog.scenes : catalog.poses).filter(name => !name.includes("deployment") && (seedKind !== "pose" || !catalog.poses[name].owner_scene)).map(name => <option key={name} value={name}>{label(name)}</option>)}</select></label><footer><button className="quiet-button" onClick={() => setCreator(false)}><ArrowLeft size={15} />Cancel</button><button className="primary-button" onClick={() => { const draft = createSceneDraft(catalog, seedKind, seed, crypto.randomUUID().slice(0, 8)); setCreator(false); onEdit(draft); }}>Create scene</button></footer></dialog>}
  </section>;
}
