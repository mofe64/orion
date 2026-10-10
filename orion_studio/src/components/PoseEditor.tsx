import { assetDisplayName } from "../lib/displayName";
import { JOINT_NAMES } from "../types";
import type { JointLimit, PoseDefinition } from "../types";

interface PoseEditorProps {
  pose: PoseDefinition;
  limits: JointLimit[];
  onChange: (pose: PoseDefinition) => void;
}

function label(name: string): string {
  return name.replace(/_joint$/, "").replaceAll("_", " ");
}

export function PoseEditor({ pose, limits, onChange }: PoseEditorProps) {
  const ranges = Object.fromEntries(limits.map((limit) => [limit.name, limit])) as Record<JointLimit["name"], JointLimit>;
  return (
    <aside className="inspector pose-inspector">
      <p className="eyebrow">Edit pose</p>
      <h2>{assetDisplayName(pose.name)}</h2>
      <div className="joint-control-list">
        {JOINT_NAMES.map((name) => {
          const range = ranges[name];
          const value = pose.positions[name];
          return <div className="joint-control" key={name}><header><label htmlFor={`pose-${name}`}>{label(name)}</label><output>{value.toFixed(3)} rad <span>{(value * 180 / Math.PI).toFixed(1)}°</span></output></header><input id={`pose-${name}`} type="range" min={range.lower_rad} max={range.upper_rad} step={0.001} value={value} onChange={(event) => onChange({ ...pose, positions: { ...pose.positions, [name]: Math.max(range.lower_rad, Math.min(range.upper_rad, Number(event.target.value))) } })} /><div><span>{range.lower_rad.toFixed(2)}</span><input aria-label={`${label(name)} radians`} type="number" min={range.lower_rad} max={range.upper_rad} step={0.001} value={value} onChange={(event) => onChange({ ...pose, positions: { ...pose.positions, [name]: Math.max(range.lower_rad, Math.min(range.upper_rad, Number(event.target.value))) } })} /><span>{range.upper_rad.toFixed(2)}</span></div></div>;
        })}
      </div>
      <p className="field-help">Ranges come from live Pi calibration when connected and the tracked accepted calibration while offline.</p>
    </aside>
  );
}
