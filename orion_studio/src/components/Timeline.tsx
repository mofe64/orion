import { useState } from "react";
import { sceneDuration, triggerTime, type SceneTrajectoryPreviews } from "../lib/preview";
import type { SceneDefinition } from "../types";

export type TrackSelection = { track: "motion" | "lighting" | "audio"; id: string };
interface TimelineProps {
  scene: SceneDefinition;
  trajectories: SceneTrajectoryPreviews;
  currentTime: number;
  selection: TrackSelection | null;
  onSelect: (selection: TrackSelection) => void;
  onTimeChange: (time: number) => void;
}
export function Timeline({ scene, trajectories, currentTime, selection, onSelect, onTimeChange }: TimelineProps) {
  const [zoom, setZoom] = useState(1);
  const duration = Math.max(1, sceneDuration(scene, trajectories));
  const rows = [
    ...scene.motion.map(event => ({ track: "motion" as const, id: event.id, label: event.play, at: event.at, duration: trajectories[event.id]?.duration_seconds ?? null, marker: undefined })),
    ...scene.lighting.map(event => ({ track: "lighting" as const, id: event.id, label: event.effect, at: triggerTime(event, scene, trajectories), duration: event.duration ?? .8, marker: event.on_marker })),
    ...scene.audio.map(event => ({ track: "audio" as const, id: event.id, label: event.cue, at: triggerTime(event, scene, trajectories), duration: null, marker: event.on_marker })),
  ];
  const selected = rows.find(row => row.id === selection?.id && row.track === selection.track);
  const pending = rows.filter(row => row.at === null || (row.track === "motion" && row.duration === null));
  const resolved = rows.filter(row => !pending.includes(row));
  return <section className="timeline" aria-label="Scene tracks">
    <header className="timeline-ruler"><strong>Scene timeline</strong><label>Zoom<select value={zoom} onChange={event => setZoom(Number(event.target.value))}><option value="1">Fit</option><option value="2">2×</option><option value="4">4×</option></select></label><span>{duration.toFixed(2)} s</span></header>
    <p className="timeline-selection">{selected ? `${selected.track} · ${selected.label.replaceAll("_", " ")} · ${selected.at === null ? `waiting for marker ${selected.marker}` : `${selected.at.toFixed(2)} s`}` : "Select an event to edit its timing and settings."}</p>
    {!!pending.length && <section className="pending-events" aria-label="Events awaiting compilation"><strong>Awaiting compilation</strong><p>These events need a compiled movement before their timing can be drawn.</p>{pending.map(row => <button key={row.id} aria-pressed={selection?.id === row.id} onClick={() => onSelect({ track: row.track, id: row.id })}>{row.label.replaceAll("_", " ")} · {row.marker ? `marker ${row.marker}` : `starts at ${row.at?.toFixed(2)} s; duration unknown`}</button>)}</section>}
    <div className="timeline-scroll"><div style={{ minWidth: `${zoom * 100}%` }}>
      {resolved.map(row => <div className="track-row" key={`${row.track}:${row.id}`}>
        <button className="track-event-label" aria-pressed={selection?.id === row.id && selection.track === row.track} onClick={() => onSelect({ track: row.track, id: row.id })}><small>{row.track}</small>{row.label.replaceAll("_", " ")}</button>
        <div className="track-canvas" onClick={event => { const rect = event.currentTarget.getBoundingClientRect(); onTimeChange(Math.max(0, Math.min(duration, (event.clientX - rect.left) / rect.width * duration))); }}>
          <button aria-label={`${row.label.replaceAll("_", " ")}, ${row.at?.toFixed(2)} seconds${row.duration === null ? ", sound duration not measured" : ""}`} aria-pressed={selection?.id === row.id} className={`track-clip ${row.track === "lighting" ? "light" : row.track} ${selection?.id === row.id ? "selected" : ""} ${row.duration === null ? "event-point" : ""}`} style={{ left: `${(row.at ?? 0) / duration * 100}%`, width: row.duration === null ? ".75rem" : `${row.duration / duration * 100}%` }} onClick={event => { event.stopPropagation(); onSelect({ track: row.track, id: row.id }); }} />
          <span className="timeline-playhead" style={{ left: `${currentTime / duration * 100}%` }} />
        </div>
      </div>)}
    </div></div>
  </section>;
}
