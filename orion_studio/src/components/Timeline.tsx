import { Pause, Play, SkipBack } from "lucide-react";
import { useRef } from "react";

import { eventDuration } from "../lib/preview";
import type { ProjectCatalog, SceneDefinition, SceneEvent } from "../types";

interface TimelineProps {
  catalog: ProjectCatalog;
  scene: SceneDefinition;
  duration: number;
  currentTime: number;
  playing: boolean;
  selectedEventId: string | null;
  onSelectEvent: (id: string) => void;
  onMoveEvent: (id: string, at: number) => void;
  onSeek: (time: number) => void;
  onTogglePlay: () => void;
  onRewind: () => void;
}

const LANES = [
  { id: "movement", label: "Movement", types: ["play_motion", "goto_pose"] },
  { id: "light", label: "Lighting", types: ["light"] },
  { id: "audio", label: "Cues", types: ["audio"] },
] as const;

function eventLabel(event: SceneEvent): string {
  switch (event.type) {
    case "play_motion": return event.motion;
    case "goto_pose": return event.pose;
    case "light": return `RGBW ${event.red}/${event.green}/${event.blue}/${event.white}`;
    case "audio": return event.cue;
  }
}

function eventClass(event: SceneEvent): string {
  if (event.type === "light") return "light";
  if (event.type === "audio") return "audio";
  return "movement";
}

export function Timeline(props: TimelineProps) {
  const {
    catalog, scene, duration, currentTime, playing, selectedEventId,
    onSelectEvent, onMoveEvent, onSeek, onTogglePlay, onRewind,
  } = props;
  const ticks = Array.from({ length: Math.ceil(duration) + 1 }, (_, index) => index);
  const drag = useRef<{
    id: string;
    pointerId: number;
    offsetSeconds: number;
    track: HTMLDivElement;
  } | null>(null);

  const seekFromPointer = (event: React.MouseEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    onSeek(Math.max(0, Math.min(duration, ((event.clientX - bounds.left) / bounds.width) * duration)));
  };

  const beginClipDrag = (pointer: React.PointerEvent<HTMLButtonElement>, event: SceneEvent) => {
    if (pointer.button !== 0) return;
    const track = pointer.currentTarget.parentElement;
    if (!(track instanceof HTMLDivElement)) return;
    const bounds = track.getBoundingClientRect();
    const pointerTime = ((pointer.clientX - bounds.left) / bounds.width) * duration;
    drag.current = {
      id: event.id,
      pointerId: pointer.pointerId,
      offsetSeconds: event.at - pointerTime,
      track,
    };
    track.setPointerCapture(pointer.pointerId);
    onSelectEvent(event.id);
    pointer.preventDefault();
  };

  const moveClip = (pointer: React.PointerEvent<HTMLDivElement>) => {
    const active = drag.current;
    if (!active || active.pointerId !== pointer.pointerId) return;
    const bounds = active.track.getBoundingClientRect();
    const pointerTime = ((pointer.clientX - bounds.left) / bounds.width) * duration;
    const at = Math.max(0, Math.min(duration, pointerTime + active.offsetSeconds));
    onMoveEvent(active.id, Number(at.toFixed(2)));
  };

  const finishClipDrag = (pointer: React.PointerEvent<HTMLDivElement>) => {
    if (drag.current?.pointerId !== pointer.pointerId) return;
    if (pointer.currentTarget.hasPointerCapture(pointer.pointerId)) {
      pointer.currentTarget.releasePointerCapture(pointer.pointerId);
    }
    drag.current = null;
  };

  return (
    <section className="timeline-panel" aria-label="Scene timeline">
      <header className="timeline-toolbar">
        <div className="transport">
          <button className="icon-button" onClick={onRewind} aria-label="Return to start"><SkipBack size={16} /></button>
          <button className="transport-play" onClick={onTogglePlay} aria-label={playing ? "Pause preview" : "Play preview"}>
            {playing ? <Pause size={16} /> : <Play size={16} />}
            {playing ? "Pause" : "Preview"}
          </button>
          <span className="timecode">{currentTime.toFixed(2)}s / {duration.toFixed(2)}s</span>
        </div>
        <p>Preview samples quintic motion locally; hardware receives only the scene name.</p>
      </header>

      <div className="timeline-grid">
        <div className="lane-label ruler-label">TRACKS</div>
        <div className="ruler" onClick={seekFromPointer}>
          {ticks.map((tick) => (
            <span key={tick} style={{ left: `${(tick / duration) * 100}%` }}>{tick}s</span>
          ))}
          <div className="playhead" style={{ left: `${(currentTime / duration) * 100}%` }} />
        </div>

        {LANES.map((lane) => (
          <div className="lane-row" key={lane.id}>
            <div className="lane-label"><span className={`lane-dot ${lane.id}`} />{lane.label}</div>
            <div
              className="lane-track"
              onClick={seekFromPointer}
              onPointerMove={moveClip}
              onPointerUp={finishClipDrag}
              onPointerCancel={finishClipDrag}
            >
              {scene.timeline
                .filter((event) => (lane.types as readonly string[]).includes(event.type))
                .map((event) => {
                  const width = Math.max(2.5, (eventDuration(event, catalog) / duration) * 100);
                  return (
                    <button
                      key={event.id}
                      className={`timeline-clip ${eventClass(event)} ${selectedEventId === event.id ? "selected" : ""}`}
                      style={{ left: `${(event.at / duration) * 100}%`, width: `${width}%` }}
                      onClick={(click) => { click.stopPropagation(); onSelectEvent(event.id); }}
                      onPointerDown={(pointer) => beginClipDrag(pointer, event)}
                      title={`${eventLabel(event)} at ${event.at.toFixed(2)} seconds`}
                    >
                      <span>{eventLabel(event)}</span>
                      <small>{event.at.toFixed(2)}s</small>
                    </button>
                  );
                })}
              <div className="playhead" style={{ left: `${(currentTime / duration) * 100}%` }} />
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}
