import {
  ChevronDown,
  MonitorPlay,
  Pause,
  Pencil,
  Play,
  Scissors,
  SkipBack,
  TimerReset,
  Trash2,
  Wifi,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { eventDuration, isDelayEvent } from "../lib/preview";
import type { ProjectCatalog, SceneDefinition, SceneEvent } from "../types";

interface TimelineProps {
  catalog: ProjectCatalog;
  scene: SceneDefinition;
  duration: number;
  currentTime: number;
  playing: boolean;
  selectedEventId: string | null;
  hardwarePreviewEnabled: boolean;
  onSelectEvent: (id: string) => void;
  onMoveEvent: (id: string, at: number) => void;
  onDeleteEvent: (id: string) => void;
  onEditPose: (id: string) => void;
  onSplitEvent: (id: string) => void;
  onInsertDelay: (id: string, seconds: number) => void;
  onSeek: (time: number) => void;
  onTogglePlay: () => void;
  onPreviewHardware: () => void;
  onRewind: () => void;
}

const LANES = [
  { id: "movement", label: "Movement", types: ["play_motion", "goto_pose", "scene"] },
  { id: "light", label: "Lighting", types: ["light"] },
  { id: "audio", label: "Cues", types: ["audio"] },
] as const;

function eventLabel(event: SceneEvent): string {
  if (isDelayEvent(event)) return "Delay";
  switch (event.type) {
    case "play_motion": return event.motion;
    case "goto_pose": return event.pose;
    case "scene": return event.scene;
    case "light": return `RGBW ${event.red}/${event.green}/${event.blue}/${event.white}`;
    case "audio": return event.cue;
  }
}

function eventClass(event: SceneEvent): string {
  if (isDelayEvent(event)) return "delay";
  switch (event.type) {
    case "play_motion": return "motion";
    case "goto_pose": return "pose";
    case "scene": return "scene-reference";
    case "light": return "light";
    case "audio": return "audio";
  }
}

interface ContextMenuState {
  id: string;
  x: number;
  y: number;
}

export function Timeline(props: TimelineProps) {
  const {
    catalog, scene, duration, currentTime, playing, selectedEventId,
    hardwarePreviewEnabled, onSelectEvent, onMoveEvent, onDeleteEvent,
    onEditPose, onSplitEvent, onInsertDelay, onSeek, onTogglePlay,
    onPreviewHardware, onRewind,
  } = props;
  const ticks = Array.from({ length: Math.ceil(duration) + 1 }, (_, index) => index);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [delaySeconds, setDelaySeconds] = useState(0.5);
  const [previewMenuOpen, setPreviewMenuOpen] = useState(false);
  const drag = useRef<{
    id: string;
    pointerId: number;
    offsetSeconds: number;
    track: HTMLDivElement;
  } | null>(null);

  useEffect(() => {
    if (!contextMenu && !previewMenuOpen) return;
    const close = () => {
      setContextMenu(null);
      setPreviewMenuOpen(false);
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
    };
  }, [contextMenu, previewMenuOpen]);

  const contextEvent = contextMenu
    ? scene.timeline.find((event) => event.id === contextMenu.id) ?? null
    : null;

  const movementDelays = (() => {
    const events = scene.timeline
      .filter((event) => ["play_motion", "goto_pose", "scene"].includes(event.type))
      .sort((left, right) => left.at - right.at);
    const delays: Array<{ id: string; at: number; duration: number }> = [];
    let cursor = 0;
    for (const event of events) {
      if (event.at > cursor + 0.01) {
        delays.push({ id: `delay:${event.id}`, at: cursor, duration: event.at - cursor });
      }
      cursor = Math.max(cursor, event.at + eventDuration(event, catalog));
    }
    return delays;
  })();

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
    setContextMenu(null);
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

  const openContextMenu = (mouse: React.MouseEvent, event: SceneEvent) => {
    mouse.preventDefault();
    mouse.stopPropagation();
    onSelectEvent(event.id);
    setDelaySeconds(0.5);
    setContextMenu({
      id: event.id,
      x: Math.min(mouse.clientX, window.innerWidth - 238),
      y: Math.min(mouse.clientY, window.innerHeight - 245),
    });
  };

  const performContextAction = (action: () => void) => {
    action();
    setContextMenu(null);
  };

  return (
    <section className="timeline-panel" aria-label="Scene timeline">
      <header className="timeline-toolbar">
        <div className="transport">
          <button className="icon-button" onClick={onRewind} aria-label="Return to start"><SkipBack size={16} /></button>
          <div className="preview-control">
            <button className="transport-play" onClick={onTogglePlay} aria-label={playing ? "Pause preview" : "Preview in Studio"}>
              {playing ? <Pause size={16} /> : <Play size={16} />}
              {playing ? "Pause" : "Preview"}
            </button>
            <button
              className="preview-menu-button"
              onPointerDown={(event) => event.stopPropagation()}
              onClick={() => setPreviewMenuOpen((open) => !open)}
              aria-label="Choose preview destination"
              aria-expanded={previewMenuOpen}
            >
              <ChevronDown size={14} />
            </button>
            {previewMenuOpen && (
              <div className="preview-menu" onPointerDown={(event) => event.stopPropagation()}>
                <button onClick={() => { setPreviewMenuOpen(false); onTogglePlay(); }}><MonitorPlay size={15} />Preview in Studio</button>
                <button disabled={!hardwarePreviewEnabled} onClick={() => { setPreviewMenuOpen(false); onPreviewHardware(); }}><Wifi size={15} />Preview on Orion</button>
              </div>
            )}
          </div>
          <span className="timecode">{currentTime.toFixed(2)}s / {duration.toFixed(2)}s</span>
        </div>
        <p>Orion keeps smooth quintic motion between every pose.</p>
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
              {lane.id === "movement" && movementDelays.map((delay) => (
                <div
                  key={delay.id}
                  className="timeline-delay"
                  style={{ left: `${(delay.at / duration) * 100}%`, width: `${(delay.duration / duration) * 100}%` }}
                  title={`Delay ${delay.duration.toFixed(2)} seconds`}
                >
                  <TimerReset size={12} /><span>Delay {delay.duration.toFixed(2)}s</span>
                </div>
              ))}
              {scene.timeline
                .filter((event) => (lane.types as readonly string[]).includes(event.type))
                .map((event) => {
                  const clipDuration = eventDuration(event, catalog);
                  const width = Math.max(2.5, (clipDuration / duration) * 100);
                  return (
                    <button
                      key={event.id}
                      className={`timeline-clip ${eventClass(event)} ${selectedEventId === event.id ? "selected" : ""}`}
                      style={{ left: `${(event.at / duration) * 100}%`, width: `${width}%` }}
                      onClick={(click) => { click.stopPropagation(); onSelectEvent(event.id); }}
                      onContextMenu={(mouse) => openContextMenu(mouse, event)}
                      onPointerDown={(pointer) => beginClipDrag(pointer, event)}
                      title={`${eventLabel(event)} at ${event.at.toFixed(2)} seconds`}
                    >
                      <span>{eventLabel(event)}</span>
                      <small>{clipDuration.toFixed(2)}s</small>
                    </button>
                  );
                })}
              <div className="playhead" style={{ left: `${(currentTime / duration) * 100}%` }} />
            </div>
          </div>
        ))}
      </div>

      {contextMenu && contextEvent && (
        <div
          className="timeline-context-menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onPointerDown={(event) => event.stopPropagation()}
          role="menu"
        >
          <strong>{eventLabel(contextEvent).replaceAll("_", " ")}</strong>
          {contextEvent.type === "goto_pose" && !isDelayEvent(contextEvent) && (
            <button onClick={() => performContextAction(() => onEditPose(contextEvent.id))}><Pencil size={14} />Edit as a new pose</button>
          )}
          {(contextEvent.type === "play_motion" || contextEvent.type === "scene") && (
            <button onClick={() => performContextAction(() => onSplitEvent(contextEvent.id))}><Scissors size={14} />Split into individual parts</button>
          )}
          <div className="delay-control">
            <label>Delay after<input type="number" min="0.01" max="60" step="0.05" value={delaySeconds} onChange={(event) => setDelaySeconds(Number(event.target.value))} /></label>
            <button disabled={!Number.isFinite(delaySeconds) || delaySeconds <= 0} onClick={() => performContextAction(() => onInsertDelay(contextEvent.id, delaySeconds))}><TimerReset size={14} />Add</button>
          </div>
          <button className="danger" onClick={() => performContextAction(() => onDeleteEvent(contextEvent.id))}><Trash2 size={14} />Delete clip</button>
        </div>
      )}
    </section>
  );
}
