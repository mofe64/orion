import { LIGHTING_EFFECTS, type LightingEffectName } from "../types";
import { StageColor } from "./StageColor";
import { CUSTOM_EFFECTS } from "../lib/lightEffects";
import { ArrowUp, ArrowDown, Trash2 } from "lucide-react";
import { useId } from "react";

import { orderedSceneMotions, moveSceneMotion } from "../lib/preview";
import { updateMovementPart } from "../lib/movementComponents";
import { assetDisplayName } from "../lib/displayName";

import type { MotionDefinition, ProjectCatalog, SceneDefinition } from "../types";
import type { TrackSelection } from "./Timeline";

interface EventInspectorProps {
  scene: SceneDefinition;
  selection: TrackSelection | null;
  catalog: ProjectCatalog;
  markers: string[];
  onChange: (scene: SceneDefinition) => void;
  onDelete: () => void;
  onChangeMovement?: (id: string, motion: MotionDefinition) => void;
  onEditPose?: (eventId: string) => void;
  onAddDelay?: () => void;
  onSplit?: (eventId: string) => void;
}

export function EventInspector({ scene, selection, catalog, markers, onChange, onDelete, onEditPose, onChangeMovement, onAddDelay, onSplit }: EventInspectorProps) {
  const brightnessHelpId = useId();
  const markerOptions = Array.from(new Set([...markers, ...scene.lighting, ...scene.audio].flatMap(value => typeof value === "string" ? [value] : value.on_marker ? [value.on_marker] : [])));
  if (!selection) return <aside className="inspector empty-inspector"><h2>Select an item</h2><p>Choose a movement, light cue, or sound to edit its timing and expression.</p></aside>;
  if (selection.track !== "motion" && selection.component?.kind === "delay") {
    const event = scene[selection.track].find(item => item.id === selection.id);
    if (!event) return null;
    return <aside className="inspector"><h2>Delay</h2><label>Wait (seconds)<input type="number" min="0" step=".05" value={event.delay ?? 0} onChange={input => onChange({ ...scene, [selection.track]: scene[selection.track].map(item => item.id === event.id ? { ...item, delay: Math.max(0,Number(input.target.value)) } : item) })} /></label><button onClick={onDelete}>Delete delay</button></aside>;
  }
  if (selection.track === "motion") {
    const event = scene.motion.find((item) => item.id === selection.id);
    if (!event) return null;
    if (selection.component && onChangeMovement) {
      const definition = catalog.motions[event.play];
      const { index, kind } = selection.component;
      const frame = definition?.keyframes[index];
      if (!frame) return null;
      const patch = (changes: Partial<typeof frame>) => onChangeMovement(event.id, updateMovementPart(definition, index, changes));
      return <aside className="inspector"><p className="eyebrow">{kind}</p><h2>{kind === "delay" ? "Delay" : assetDisplayName(frame.pose ?? "Relative pose")}</h2>
        {kind === "pose" && frame.pose && <label>Pose<select value={frame.pose} onChange={input => patch({ pose: input.target.value })}>{Object.keys(catalog.poses).filter(name => name !== "rest").map(name => <option key={name} value={name}>{assetDisplayName(name)}</option>)}</select></label>}
        <label>{kind === "delay" ? "Delay (s)" : "Move duration (s)"}<input type="number" min={kind === "delay" ? 0 : .02} step={.05} value={kind === "delay" ? frame.hold : frame.duration} onChange={input => { const value = Number(input.target.value); if (Number.isFinite(value) && value >= (kind === "delay" ? 0 : .02)) patch(kind === "delay" ? { hold: value } : { duration: value }); }} /></label>
        {kind === "pose" && (frame.arrival === "settle" ? <label>Pause after (s)<input type="number" min="0" step=".05" value={frame.hold} onChange={input => { const hold = Number(input.target.value); if (Number.isFinite(hold) && hold >= 0) patch({ hold }); }} /></label> : <p className="field-help">Continues smoothly</p>)}
        {kind === "pose" && onEditPose && <button className="secondary-button" onClick={() => onEditPose(event.id)}>Edit pose</button>}
        {kind === "pose" && onAddDelay && <button className="quiet-button" onClick={onAddDelay}>Add delay after pose</button>}
        <button className="danger-button" onClick={onDelete}><Trash2 size={15} />Delete {kind}</button>
      </aside>;
    }
    const ordered = orderedSceneMotions(scene);
    const index = ordered.findIndex(item => item.id === event.id);
    return <aside className="inspector"><h2>{assetDisplayName(event.play)}</h2><label>Movement<select value={event.play} onChange={(input) => onChange({ ...scene, motion: scene.motion.map((item) => item.id === event.id ? { ...item, play: input.target.value } : item) })}>{Object.keys(catalog.motions).map((name) => <option key={name} value={name}>{assetDisplayName(name)}</option>)}</select></label><p>Movement {index + 1} of {ordered.length}. Orion calculates the timing automatically.</p><div className="field-grid"><button className="quiet-button" disabled={index === 0} onClick={() => onChange(moveSceneMotion(scene, event.id, -1))}><ArrowUp size={15} />Move earlier</button><button className="quiet-button" disabled={index === ordered.length - 1} onClick={() => onChange(moveSceneMotion(scene, event.id, 1))}><ArrowDown size={15} />Move later</button></div>{onEditPose && catalog.motions[event.play]?.space === "absolute" && <button className="secondary-button" onClick={() => onEditPose(event.id)}>Edit destination pose</button>}{onSplit && !event.show_parts && <button className="secondary-button" onClick={() => onSplit(event.id)}>Split into poses</button>}{onAddDelay && <button className="quiet-button" onClick={onAddDelay}>Add delay after movement</button>}<button className="danger-button" onClick={onDelete}><Trash2 size={15} />Delete movement</button></aside>;
  }
  if (selection.track === "lighting") {
    const event = scene.lighting.find((item) => item.id === selection.id);
    if (!event) return null;
    const patch = (changes: Partial<typeof event>) => onChange({ ...scene, lighting: scene.lighting.map((item) => item.id === event.id ? { ...item, ...changes } : item) });
    const effect = CUSTOM_EFFECTS[event.effect as keyof typeof CUSTOM_EFFECTS];
    return <aside className="inspector"><p className="eyebrow">Light</p><label>Effect<select value={event.effect} onChange={input => {
      const name = input.target.value as LightingEffectName;
      const custom = CUSTOM_EFFECTS[name as keyof typeof CUSTOM_EFFECTS];
      patch({ effect: name, transition: 0, period: undefined, palette: undefined,
        levels: custom ? name === "pulse" || name === "breathe" ? [.15,1] : [1,1] : undefined,
        colors: custom ? custom.stages.map((_,index) => event.colors?.[index] ?? "warm_white") : undefined });
    }}>
      <optgroup label="Custom effects">{Object.entries(CUSTOM_EFFECTS).map(([key,value]) => <option key={key} value={key}>{value.label}</option>)}</optgroup>
      <optgroup label="Orion presets">{LIGHTING_EFFECTS.map(name => <option key={name} value={name}>{assetDisplayName(name)}</option>)}</optgroup>
    </select></label>
    <p className="field-help" id={brightnessHelpId}>{effect ? "Stage brightness sets each part of the effect. Overall brightness scales them together." : "Overall brightness scales the preset without changing its colors or rhythm."}</p>
    <details className="effect-help"><summary>How this effect works</summary><p>{effect?.help ?? "An Orion character preset with its original color and rhythm. Choose another effect to customize its colors."}</p></details>
    {effect && <div className="stage-colors">{effect.stages.map((stage,index) => <div key={stage}><StageColor label={stage} value={event.colors?.[index] ?? "warm_white"} onChange={color => { const colors = effect.stages.map((_,i) => event.colors?.[i] ?? "warm_white"); colors[index] = color; patch({ colors }); }} /><label><span className="lighting-brightness-label">{stage} brightness<output>{Math.round((event.levels?.[index] ?? (["pulse","breathe"].includes(event.effect) && index === 0 ? .15 : 1))*100)}%</output></span><input type="range" aria-label={`${stage} brightness`} aria-valuetext={`${Math.round((event.levels?.[index] ?? (["pulse","breathe"].includes(event.effect) && index === 0 ? .15 : 1))*100)}%`} min="0" max="1" step=".01" value={event.levels?.[index] ?? (["pulse","breathe"].includes(event.effect) && index === 0 ? .15 : 1)} onChange={input => { const levels = effect.stages.map((_,i) => event.levels?.[i] ?? (["pulse","breathe"].includes(event.effect) && i === 0 ? .15 : 1)); levels[index] = Number(input.target.value); patch({ levels }); }} /></label></div>)}</div>}
    <label><span className="lighting-brightness-label">Overall brightness<output>{Math.round((event.intensity ?? 1)*100)}%</output></span><input aria-label="Overall brightness" aria-describedby={brightnessHelpId} aria-valuetext={`${Math.round((event.intensity ?? 1)*100)}%`} type="range" min="0" max="1" step=".01" value={event.intensity ?? 1} onChange={input => patch({ intensity: Number(input.target.value) })} /></label>
    {(["pulse","breathe"].includes(event.effect)) && <label>Cycle length (seconds)<input type="number" min=".1" max="60" step=".1" value={event.period ?? 2} onChange={input => patch({ period: Math.max(.1,Number(input.target.value)) })} /></label>}
    <TimingFields event={event} markers={markerOptions} onChange={patch} />
    <label>Lasts (seconds)<input type="number" min=".05" step=".05" value={event.duration ?? .8} onChange={input => patch({ duration: Math.max(.05,Number(input.target.value)) })} /></label>
    <button className="danger-button" onClick={onDelete}><Trash2 size={15} />Delete light</button></aside>;
  }
  const event = scene.audio.find((item) => item.id === selection.id);
  if (!event) return null;
  const patch = (changes: Partial<typeof event>) => onChange({ ...scene, audio: scene.audio.map((item) => item.id === event.id ? { ...item, ...changes } : item) });
  return <aside className="inspector"><p className="eyebrow">Sound</p><h2>{assetDisplayName(event.cue)}</h2><label>Sound<select value={event.cue} onChange={(input) => patch({ cue: input.target.value })}>{catalog.cues.map((cue) => <option key={cue} value={cue}>{assetDisplayName(cue)}</option>)}</select></label><TimingFields event={event} markers={markerOptions} onChange={patch} /><button className="danger-button" onClick={onDelete}><Trash2 size={15} />Delete sound</button></aside>;
}

function TimingFields({ event, markers, onChange }: { event: import("../types").TrackTiming; markers: string[]; onChange: (value: Partial<import("../types").TrackTiming>) => void }) {
  return <><label>Starts<select value={event.after_previous ? "after" : event.on_marker ? `marker:${event.on_marker}` : "time"} onChange={input => onChange({ resolved_at: undefined, after_previous: input.target.value === "after" || undefined, at: input.target.value.startsWith("marker:") ? undefined : 0, on_marker: input.target.value.startsWith("marker:") ? input.target.value.slice(7) : undefined })}>
    <option value="after">After previous item</option><option value="time">At a scene time</option>{markers.map(marker => <option key={marker} value={`marker:${marker}`}>At movement cue: {marker.replaceAll("_", " ")}</option>)}
  </select></label>
  <details className="effect-help"><summary>About start options</summary><p>After previous item keeps this track in order. A scene time counts from playback start. A movement cue follows a named moment in a movement, even if the movement slows down. An item waits if this track is still busy.</p></details>
  {!event.after_previous && !event.on_marker && <label>Scene time (seconds)<input type="number" min="0" step=".05" value={event.at ?? 0} onChange={input => onChange({ at: Math.max(0,Number(input.target.value)), resolved_at: undefined })} /></label>}
  <label>Delay before (seconds)<input type="number" min="0" step=".05" value={event.delay ?? 0} onChange={input => onChange({ delay: Math.max(0,Number(input.target.value)), resolved_at: undefined })} /></label></>;
}
