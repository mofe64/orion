import { ChevronDown, ChevronUp, Plus, Save, Trash2 } from "lucide-react";

import type { MotionDefinition, MotionKeyframe, ProjectCatalog } from "../types";

interface MotionEditorProps {
  motion: MotionDefinition;
  catalog: ProjectCatalog;
  saveAsName: string;
  dirty: boolean;
  onSaveAsNameChange: (name: string) => void;
  onDescriptionChange: (description: string) => void;
  onKeyframesChange: (keyframes: MotionKeyframe[]) => void;
  onSave: () => void;
}

export function MotionEditor({
  motion,
  catalog,
  saveAsName,
  dirty,
  onSaveAsNameChange,
  onDescriptionChange,
  onKeyframesChange,
  onSave,
}: MotionEditorProps) {
  const patchFrame = (index: number, changes: Partial<MotionKeyframe>) => {
    onKeyframesChange(motion.keyframes.map((frame, frameIndex) => (
      frameIndex === index ? { ...frame, ...changes } : frame
    )));
  };
  const moveFrame = (index: number, direction: -1 | 1) => {
    const target = index + direction;
    if (target < 0 || target >= motion.keyframes.length) return;
    const next = [...motion.keyframes];
    [next[index], next[target]] = [next[target], next[index]];
    onKeyframesChange(next);
  };
  const addFrame = () => {
    onKeyframesChange([
      ...motion.keyframes,
      { pose: Object.keys(catalog.poses)[0], duration: 1.5, hold: 0 },
    ]);
  };
  const removeFrame = (index: number) => {
    if (motion.keyframes.length === 1) return;
    onKeyframesChange(motion.keyframes.filter((_, frameIndex) => frameIndex !== index));
  };

  return (
    <aside className="inspector motion-editor">
      <div className="inspector-heading">
        <div>
          <p className="panel-kicker">QUINTIC MOTION</p>
          <h2>{motion.name.replaceAll("_", " ")}</h2>
        </div>
        {dirty && <span className="dirty-badge">Unsaved</span>}
      </div>

      <div className="inspector-fields">
        <label>
          Save As name
          <input value={saveAsName} onChange={(event) => onSaveAsNameChange(event.target.value)} />
        </label>
        <label>
          Description
          <textarea
            rows={3}
            value={motion.description}
            onChange={(event) => onDescriptionChange(event.target.value)}
          />
        </label>

        <div className="keyframe-list">
          {motion.keyframes.map((frame, index) => (
            <section className="keyframe-card" key={`${index}-${frame.pose}`}>
              <header>
                <strong>Frame {index + 1}</strong>
                <div>
                  <button className="icon-button" onClick={() => moveFrame(index, -1)} disabled={index === 0} aria-label={`Move frame ${index + 1} up`}><ChevronUp size={13} /></button>
                  <button className="icon-button" onClick={() => moveFrame(index, 1)} disabled={index === motion.keyframes.length - 1} aria-label={`Move frame ${index + 1} down`}><ChevronDown size={13} /></button>
                  <button className="icon-button danger" onClick={() => removeFrame(index)} disabled={motion.keyframes.length === 1} aria-label={`Delete frame ${index + 1}`}><Trash2 size={13} /></button>
                </div>
              </header>
              <label>
                Named pose
                <select value={frame.pose} onChange={(event) => patchFrame(index, { pose: event.target.value })}>
                  {Object.keys(catalog.poses).map((name) => <option key={name}>{name}</option>)}
                </select>
              </label>
              <div className="keyframe-timing">
                <label>
                  Transition
                  <input type="number" min={0.001} max={300} step={0.05} value={frame.duration} onChange={(event) => patchFrame(index, { duration: Number(event.target.value) })} />
                </label>
                <label>
                  Hold
                  <input type="number" min={0} max={300} step={0.05} value={frame.hold} onChange={(event) => patchFrame(index, { hold: Number(event.target.value) })} />
                </label>
              </div>
            </section>
          ))}
        </div>

        <button className="secondary-button" onClick={addFrame}><Plus size={14} />Add keyframe</button>
        <button className="primary-button" onClick={onSave}><Save size={15} />Save as new motion</button>
        <p className="field-help">Orion computes quintic interpolation between these named poses.</p>
      </div>
    </aside>
  );
}
