import { Lightbulb, Move3d, Music2 } from "lucide-react";

import { sceneDuration, sceneMarkers, triggerTime } from "../lib/preview";
import type { SceneTrajectoryPreviews } from "../lib/preview";
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

function percent(time: number, duration: number): string {
  return `${Math.max(0, Math.min(100, time / duration * 100))}%`;
}

export function Timeline({ scene, trajectories, currentTime, selection, onSelect, onTimeChange }: TimelineProps) {
  const duration = sceneDuration(scene, trajectories);
  const clickTime = (event: React.MouseEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    onTimeChange((event.clientX - bounds.left) / bounds.width * duration);
  };
  return (
    <section className="timeline" aria-label="Scene tracks">
      <header className="timeline-ruler">
        <span>0:00</span><strong>Parallel scene tracks</strong><span>{duration.toFixed(2)} s</span>
      </header>
      <div className="track-row">
        <div className="track-label"><Move3d size={15} /><span>Motion</span></div>
        <div className="track-canvas" onClick={clickTime}>
          {scene.motion.map((event) => (
            <button key={event.id} className={`track-clip motion ${selection?.id === event.id ? "selected" : ""}`} style={{ left: percent(event.at, duration), width: percent(trajectories[event.id]?.duration_seconds ?? 0, duration) }} onClick={(click) => { click.stopPropagation(); onSelect({ track: "motion", id: event.id }); }}>
              <strong>{event.play.replaceAll("_", " ")}</strong><span>{event.at.toFixed(2)} s</span>
            </button>
          ))}
          {sceneMarkers(scene, trajectories).map((marker) => <span key={`${marker.motion_event_id}:${marker.name}`} className="timeline-marker" style={{ left: percent(marker.time_seconds, duration) }} title={`${marker.name} · ${marker.time_seconds.toFixed(2)} s`} />)}
          <span className="timeline-playhead" style={{ left: percent(currentTime, duration) }} />
        </div>
      </div>
      <div className="track-row">
        <div className="track-label"><Lightbulb size={15} /><span>Light</span></div>
        <div className="track-canvas" onClick={clickTime}>
          {scene.lighting.map((event) => {
            const at = triggerTime(event, scene, trajectories) ?? 0;
            return <button key={event.id} className={`track-clip light ${selection?.id === event.id ? "selected" : ""}`} style={{ left: percent(at, duration), width: percent(event.duration ?? 0.8, duration) }} onClick={(click) => { click.stopPropagation(); onSelect({ track: "lighting", id: event.id }); }}><strong>{event.effect.replaceAll("_", " ")}</strong><span>{event.on_marker ? `@${event.on_marker}` : `${at.toFixed(2)} s`}</span></button>;
          })}
          <span className="timeline-playhead" style={{ left: percent(currentTime, duration) }} />
        </div>
      </div>
      <div className="track-row">
        <div className="track-label"><Music2 size={15} /><span>Sound</span></div>
        <div className="track-canvas" onClick={clickTime}>
          {scene.audio.map((event) => {
            const at = triggerTime(event, scene, trajectories) ?? 0;
            return <button key={event.id} className={`track-clip audio ${selection?.id === event.id ? "selected" : ""}`} style={{ left: percent(at, duration), width: "max(4%, 5rem)" }} onClick={(click) => { click.stopPropagation(); onSelect({ track: "audio", id: event.id }); }}><strong>{event.cue.replaceAll("_", " ")}</strong><span>{event.on_marker ? `@${event.on_marker}` : `${at.toFixed(2)} s`}</span></button>;
          })}
          <span className="timeline-playhead" style={{ left: percent(currentTime, duration) }} />
        </div>
      </div>
    </section>
  );
}
