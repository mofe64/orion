import { Trash2 } from "lucide-react";

import { LIGHTING_EFFECTS } from "../types";
import type { ProjectCatalog, SceneDefinition } from "../types";
import type { TrackSelection } from "./Timeline";

interface EventInspectorProps {
  scene: SceneDefinition;
  selection: TrackSelection | null;
  catalog: ProjectCatalog;
  markers: string[];
  onChange: (scene: SceneDefinition) => void;
  onDelete: () => void;
}

export function EventInspector({ scene, selection, catalog, markers, onChange, onDelete }: EventInspectorProps) {
  const markerOptions = Array.from(new Set([...markers, ...scene.lighting, ...scene.audio].flatMap(value => typeof value === "string" ? [value] : value.on_marker ? [value.on_marker] : [])));
  if (!selection) return <aside className="inspector empty-inspector"><p className="eyebrow">Inspector</p><h2>Select a track event</h2><p>Choose a motion, light, or sound event to edit its timing and expression.</p></aside>;
  if (selection.track === "motion") {
    const event = scene.motion.find((item) => item.id === selection.id);
    if (!event) return null;
    return <aside className="inspector"><p className="eyebrow">Motion event</p><h2>{event.play.replaceAll("_", " ")}</h2><label>Motion<select value={event.play} onChange={(input) => onChange({ ...scene, motion: scene.motion.map((item) => item.id === event.id ? { ...item, play: input.target.value } : item) })}>{Object.keys(catalog.motions).map((name) => <option key={name}>{name}</option>)}</select></label><label>Start time<input type="number" min={0} step={0.05} value={event.at} onChange={(input) => onChange({ ...scene, motion: scene.motion.map((item) => item.id === event.id ? { ...item, at: Number(input.target.value) } : item) })} /></label><button className="danger-button" onClick={onDelete}><Trash2 size={15} />Delete motion event</button></aside>;
  }
  if (selection.track === "lighting") {
    const event = scene.lighting.find((item) => item.id === selection.id);
    if (!event) return null;
    const patch = (changes: Partial<typeof event>) => onChange({ ...scene, lighting: scene.lighting.map((item) => item.id === event.id ? { ...item, ...changes } : item) });
    return <aside className="inspector"><p className="eyebrow">Lighting event</p><h2>{event.effect.replaceAll("_", " ")}</h2><label>Effect<select value={event.effect} onChange={(input) => patch({ effect: input.target.value as typeof event.effect })}>{LIGHTING_EFFECTS.map((name) => <option key={name}>{name}</option>)}</select></label><label>Trigger<select value={event.on_marker ? `marker:${event.on_marker}` : "time"} onChange={(input) => input.target.value === "time" ? patch({ at: 0, on_marker: undefined }) : patch({ at: undefined, on_marker: input.target.value.slice(7) })}><option value="time">At a time</option>{markerOptions.map((marker) => <option value={`marker:${marker}`} key={marker}>At marker · {marker}{markers.includes(marker) ? "" : " (unresolved)"}</option>)}</select></label>{event.on_marker === undefined && <label>Start time<input type="number" min={0} step={0.05} value={event.at ?? 0} onChange={(input) => patch({ at: Number(input.target.value) })} /></label>}<label>Duration<input type="number" min={0} step={0.05} value={event.duration ?? 0.8} onChange={(input) => patch({ duration: Number(input.target.value) })} /></label><button className="danger-button" onClick={onDelete}><Trash2 size={15} />Delete lighting event</button></aside>;
  }
  const event = scene.audio.find((item) => item.id === selection.id);
  if (!event) return null;
  const patch = (changes: Partial<typeof event>) => onChange({ ...scene, audio: scene.audio.map((item) => item.id === event.id ? { ...item, ...changes } : item) });
  return <aside className="inspector"><p className="eyebrow">Sound event</p><h2>{event.cue.replaceAll("_", " ")}</h2><label>Warm cue<select value={event.cue} onChange={(input) => patch({ cue: input.target.value })}>{catalog.cues.map((cue) => <option key={cue}>{cue}</option>)}</select></label><label>Trigger<select value={event.on_marker ? `marker:${event.on_marker}` : "time"} onChange={(input) => input.target.value === "time" ? patch({ at: 0, on_marker: undefined }) : patch({ at: undefined, on_marker: input.target.value.slice(7) })}><option value="time">At a time</option>{markerOptions.map((marker) => <option value={`marker:${marker}`} key={marker}>At marker · {marker}{markers.includes(marker) ? "" : " (unresolved)"}</option>)}</select></label>{event.on_marker === undefined && <label>Start time<input type="number" min={0} step={0.05} value={event.at ?? 0} onChange={(input) => patch({ at: Number(input.target.value) })} /></label>}<button className="danger-button" onClick={onDelete}><Trash2 size={15} />Delete sound event</button></aside>;
}
