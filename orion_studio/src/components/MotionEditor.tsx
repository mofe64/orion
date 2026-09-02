import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-react";

import { JOINT_NAMES, MOTION_STYLES } from "../types";
import type { JointName, MotionDefinition, MotionKeyframe, ProjectCatalog } from "../types";

interface MotionEditorProps {
  motion: MotionDefinition;
  catalog: ProjectCatalog;
  onChange: (motion: MotionDefinition) => void;
}

export function MotionEditor({ motion, catalog, onChange }: MotionEditorProps) {
  const updateFrames = (keyframes: MotionKeyframe[]) => onChange({ ...motion, keyframes });
  const patch = (index: number, changes: Partial<MotionKeyframe>) => updateFrames(motion.keyframes.map((frame, frameIndex) => frameIndex === index ? { ...frame, ...changes } : frame));
  const move = (index: number, direction: -1 | 1) => {
    const target = index + direction;
    if (target < 0 || target >= motion.keyframes.length) return;
    const frames = [...motion.keyframes];
    [frames[index], frames[target]] = [frames[target], frames[index]];
    updateFrames(frames);
  };
  const add = () => updateFrames([...motion.keyframes, motion.space === "absolute"
    ? { pose: Object.keys(catalog.poses)[0], duration: 0.5, arrival: "settle", hold: 0 }
    : { offsets: {}, duration: 0.5, arrival: "settle", hold: 0 }]);

  return (
    <aside className="inspector motion-inspector">
      <p className="eyebrow">Motion v2</p>
      <h2>{motion.name.replaceAll("_", " ")}</h2>
      <label>Description<textarea rows={3} value={motion.description} onChange={(event) => onChange({ ...motion, description: event.target.value })} /></label>
      <div className="field-grid">
        <label>Space<select value={motion.space} onChange={(event) => {
          const space = event.target.value as MotionDefinition["space"];
          onChange({ ...motion, space, return_to_anchor: space === "anchor_relative", keyframes: motion.keyframes.map((frame) => space === "absolute" ? { pose: frame.pose ?? Object.keys(catalog.poses)[0], duration: frame.duration, arrival: frame.arrival, hold: frame.hold, marker: frame.marker } : { offsets: frame.offsets ?? {}, duration: frame.duration, arrival: frame.arrival, hold: frame.hold, marker: frame.marker }) });
        }}><option value="absolute">Absolute poses</option><option value="anchor_relative">Anchor relative</option></select></label>
        <label>Character style<select value={motion.style} onChange={(event) => onChange({ ...motion, style: event.target.value as MotionDefinition["style"] })}>{MOTION_STYLES.map((style) => <option key={style}>{style}</option>)}</select></label>
      </div>
      <div className="keyframe-list">
        {motion.keyframes.map((frame, index) => (
          <section className={`keyframe-card ${frame.arrival}`} key={`${index}-${frame.pose ?? "relative"}`}>
            <header><div><span>Keyframe {index + 1}</span><strong>{frame.arrival === "through" ? "Flows through" : "Settles here"}</strong></div><div><button className="icon-button" disabled={index === 0} onClick={() => move(index, -1)} aria-label={`Move keyframe ${index + 1} earlier`}><ArrowUp size={14} /></button><button className="icon-button" disabled={index === motion.keyframes.length - 1} onClick={() => move(index, 1)} aria-label={`Move keyframe ${index + 1} later`}><ArrowDown size={14} /></button><button className="icon-button danger" disabled={motion.keyframes.length === 1} onClick={() => updateFrames(motion.keyframes.filter((_, frameIndex) => frameIndex !== index))} aria-label={`Delete keyframe ${index + 1}`}><Trash2 size={14} /></button></div></header>
            {motion.space === "absolute" ? <label>Named pose<select value={frame.pose} onChange={(event) => patch(index, { pose: event.target.value })}>{Object.keys(catalog.poses).map((name) => <option key={name}>{name}</option>)}</select></label> : <div className="offset-grid">{JOINT_NAMES.map((name: JointName) => <label key={name}>{name.replace(/_joint$/, "").replaceAll("_", " ")}<input type="number" step={0.01} value={frame.offsets?.[name] ?? 0} onChange={(event) => patch(index, { offsets: { ...frame.offsets, [name]: Number(event.target.value) } })} /></label>)}</div>}
            <div className="field-grid three"><label>Duration<input type="number" min={0.02} step={0.05} value={frame.duration} onChange={(event) => patch(index, { duration: Number(event.target.value) })} /></label><label>Arrival<select value={frame.arrival} disabled={index === motion.keyframes.length - 1} onChange={(event) => patch(index, { arrival: event.target.value as MotionKeyframe["arrival"], ...(event.target.value === "through" ? { hold: 0 } : {}) })}><option value="through">Through</option><option value="settle">Settle</option></select></label><label>Hold<input type="number" min={0} step={0.05} disabled={frame.arrival === "through"} value={frame.hold} onChange={(event) => patch(index, { hold: Number(event.target.value) })} /></label></div>
            <label>Marker<input value={frame.marker ?? ""} placeholder="Optional semantic marker" onChange={(event) => patch(index, { marker: event.target.value || undefined })} /></label>
          </section>
        ))}
      </div>
      <button className="secondary-button" onClick={add}><Plus size={15} />Add keyframe</button>
      <p className="field-help">Through keyframes preserve continuous position, velocity, and acceleration. Holds are available only after a settle.</p>
    </aside>
  );
}
