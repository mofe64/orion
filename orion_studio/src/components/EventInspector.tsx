import { Trash2 } from "lucide-react";

import type { ProjectCatalog, SceneEvent } from "../types";

interface EventInspectorProps {
  catalog: ProjectCatalog;
  event: SceneEvent | null;
  onChange: (event: SceneEvent) => void;
  onDelete: () => void;
}

function NumberField(props: {
  label: string;
  value: number;
  min?: number;
  max?: number;
  step?: number;
  onChange: (value: number) => void;
}) {
  return (
    <label>
      {props.label}
      <input
        type="number"
        value={props.value}
        min={props.min}
        max={props.max}
        step={props.step ?? 0.01}
        onChange={(event) => props.onChange(Number(event.target.value))}
      />
    </label>
  );
}

export function EventInspector({ catalog, event, onChange, onDelete }: EventInspectorProps) {
  if (!event) {
    return (
      <aside className="inspector empty-inspector">
        <p className="panel-kicker">INSPECTOR</p>
        <h2>Select a timeline clip</h2>
        <p>Choose a movement, lighting transition, or cue to edit its semantic fields.</p>
      </aside>
    );
  }

  const patch = (changes: Partial<SceneEvent>) => onChange({ ...event, ...changes } as SceneEvent);

  return (
    <aside className="inspector">
      <div className="inspector-heading">
        <div>
          <p className="panel-kicker">INSPECTOR</p>
          <h2>{event.type.replaceAll("_", " ")}</h2>
        </div>
        <button className="icon-button danger" onClick={onDelete} aria-label="Delete selected event"><Trash2 size={16} /></button>
      </div>

      <div className="inspector-fields">
        <NumberField label="Start time" value={event.at} min={0} max={300} onChange={(at) => patch({ at })} />

        {event.type === "play_motion" && (
          <label>
            Authored motion
            <select value={event.motion} onChange={(change) => patch({ motion: change.target.value })}>
              {Object.keys(catalog.motions).map((name) => <option key={name}>{name}</option>)}
            </select>
          </label>
        )}

        {event.type === "goto_pose" && (
          <>
            <label>
              Named pose
              <select value={event.pose} onChange={(change) => patch({ pose: change.target.value })}>
                {Object.keys(catalog.poses).map((name) => <option key={name}>{name}</option>)}
              </select>
            </label>
            <NumberField label="Move duration" value={event.duration_seconds} min={0.1} max={60} onChange={(duration_seconds) => patch({ duration_seconds })} />
          </>
        )}

        {event.type === "light" && (
          <>
            <div className="channel-grid">
              <NumberField label="Red" value={event.red} min={0} max={255} step={1} onChange={(red) => patch({ red })} />
              <NumberField label="Green" value={event.green} min={0} max={255} step={1} onChange={(green) => patch({ green })} />
              <NumberField label="Blue" value={event.blue} min={0} max={255} step={1} onChange={(blue) => patch({ blue })} />
              <NumberField label="White" value={event.white} min={0} max={255} step={1} onChange={(white) => patch({ white })} />
            </div>
            <NumberField label="Fade duration" value={event.transition_seconds} min={0} max={60} onChange={(transition_seconds) => patch({ transition_seconds })} />
          </>
        )}

        {event.type === "audio" && (
          <label>
            Named cue
            <select value={event.cue} onChange={(change) => patch({ cue: change.target.value })}>
              {catalog.cues.map((name) => <option key={name}>{name}</option>)}
            </select>
          </label>
        )}
      </div>
    </aside>
  );
}
