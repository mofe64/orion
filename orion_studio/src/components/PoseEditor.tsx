import { Check, Save } from "lucide-react";

import { JOINT_NAMES } from "../types";
import type { JointLimit, PoseDefinition } from "../types";

interface PoseEditorProps {
  pose: PoseDefinition;
  limits: JointLimit[];
  saveAsName: string;
  dirty: boolean;
  sceneDraft?: boolean;
  onSaveAsNameChange: (name: string) => void;
  onDescriptionChange: (description: string) => void;
  onPositionChange: (joint: JointLimit["name"], value: number) => void;
  onSave: () => void;
}

function jointLabel(name: string): string {
  return name.replace(/_joint$/, "").replaceAll("_", " ");
}

export function PoseEditor({
  pose,
  limits,
  saveAsName,
  dirty,
  sceneDraft = false,
  onSaveAsNameChange,
  onDescriptionChange,
  onPositionChange,
  onSave,
}: PoseEditorProps) {
  const ranges = Object.fromEntries(limits.map((limit) => [limit.name, limit])) as Record<
    JointLimit["name"],
    JointLimit
  >;

  return (
    <aside className="inspector pose-editor">
      <div className="inspector-heading">
        <div>
          <p className="panel-kicker">KEYFRAME POSE</p>
          <h2>{sceneDraft ? "Scene pose" : pose.name.replaceAll("_", " ")}</h2>
        </div>
        {dirty && <span className="dirty-badge">Unsaved</span>}
      </div>

      <div className="inspector-fields">
        {!sceneDraft && (
          <label>
            Save As name
            <input value={saveAsName} onChange={(event) => onSaveAsNameChange(event.target.value)} />
          </label>
        )}
        <label>
          Description
          <textarea
            rows={3}
            value={pose.description}
            onChange={(event) => onDescriptionChange(event.target.value)}
          />
        </label>

        <div className="joint-control-list">
          {JOINT_NAMES.map((name) => {
            const limit = ranges[name];
            if (!limit) return null;
            const value = pose.positions[name];
            return (
              <div className="joint-control" key={name}>
                <div>
                  <label htmlFor={`joint-${name}`}>{jointLabel(name)}</label>
                  <output>{value.toFixed(3)} rad · {(value * 180 / Math.PI).toFixed(1)}°</output>
                </div>
                <input
                  id={`joint-${name}`}
                  type="range"
                  min={limit.lower_rad}
                  max={limit.upper_rad}
                  step={0.001}
                  value={value}
                  onChange={(event) => onPositionChange(name, Number(event.target.value))}
                />
                <div className="joint-range">
                  <span>{limit.lower_rad.toFixed(2)}</span>
                  <input
                    aria-label={`${jointLabel(name)} radians`}
                    type="number"
                    min={limit.lower_rad}
                    max={limit.upper_rad}
                    step={0.001}
                    value={value}
                    onChange={(event) => onPositionChange(name, Number(event.target.value))}
                  />
                  <span>{limit.upper_rad.toFixed(2)}</span>
                </div>
              </div>
            );
          })}
        </div>

        <button className="primary-button" onClick={onSave}>
          {sceneDraft ? <Check size={15} /> : <Save size={15} />}
          {sceneDraft ? "Complete edit" : "Save as new pose"}
        </button>
        <p className="field-help">{sceneDraft
          ? "Complete edit keeps this pose inside the scene draft. It is named and saved with the scene later."
          : "Saving creates a named keyframe. Slider changes never move Orion."}</p>
      </div>
    </aside>
  );
}
