import { useEffect, useRef, useState } from "react";
import { sceneDuration, triggerTime, type SceneTrajectoryPreviews } from "../lib/preview";
import { movementComponents, type MovementComponent } from "../lib/movementComponents";
import type { MotionDefinition, ProjectCatalog, SceneDefinition } from "../types";

export type TrackSelection = { track: "motion" | "lighting" | "audio"; id: string; component?: MovementComponent };
interface TimelineProps {
  scene: SceneDefinition;
  catalog?: ProjectCatalog;
  onMove?: (selection: TrackSelection, time: number) => void;
  onEditPose?: (selection: TrackSelection) => void;
  onDelete?: (selection: TrackSelection) => void;
  onToggleParts?: (eventId: string) => void;
  onChangeMovement?: (eventId: string, motion: MotionDefinition) => void;
  trajectories: SceneTrajectoryPreviews;
  currentTime: number;
  selection: TrackSelection | null;
  onSelect: (selection: TrackSelection) => void;
  onTimeChange: (time: number) => void;
}
export function Timeline({ scene, catalog, onChangeMovement, onToggleParts, onDelete, onMove, onEditPose, trajectories, currentTime, selection, onSelect, onTimeChange }: TimelineProps) {
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const drag = useRef<{ x: number; time: number; target: TrackSelection; width: number; element: HTMLElement } | null>(null);
  const menuContainer = useRef<HTMLDivElement>(null);
  const [zoom, setZoom] = useState(1);
  const [menu, setMenu] = useState<TrackSelection & { x: number; y: number } | null>(null);
  const revealSplit = useRef<string | null>(null);
  const firstComponents = useRef<Record<string, HTMLButtonElement | null>>({});
  useEffect(() => {
    const id = revealSplit.current;
    if (id && scene.motion.find(event => event.id === id)?.show_parts) {
      firstComponents.current[id]?.scrollIntoView({ block: "nearest" });
      firstComponents.current[id]?.focus({ preventScroll: true });
      revealSplit.current = null;
    }
  }, [scene]);
  useEffect(() => {
    if (!menu) return;
    const dismissOutside = (event: PointerEvent) => {
      if (!menuContainer.current?.contains(event.target as Node)) setMenu(null);
    };
    document.addEventListener("pointerdown", dismissOutside, true);
    return () => document.removeEventListener("pointerdown", dismissOutside, true);
  }, [menu]);
  const menuRef = useRef<HTMLButtonElement>(null);
  useEffect(() => { if (menu) menuRef.current?.focus(); }, [menu]);
  const openMenu = (target: TrackSelection, x: number, y: number) => {
    onSelect(target);
    setMenu({ ...target, x: Math.max(8, Math.min(x, window.innerWidth - 260)), y: Math.max(8, Math.min(y, window.innerHeight - 120)) });
  };
  const contextHandlers = (target: TrackSelection) => ({
    onContextMenu: (event: React.MouseEvent<HTMLElement>) => { event.preventDefault(); event.stopPropagation(); openMenu(target, event.clientX, event.clientY); },
    onKeyDown: (event: React.KeyboardEvent<HTMLElement>) => {
      if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
        event.preventDefault(); const rect = event.currentTarget.getBoundingClientRect(); openMenu(target, rect.left, rect.top);
      }
    },
  });
  const duration = Math.max(1, sceneDuration(scene, trajectories));
  type Row = TrackSelection & { label: string; at: number | null; duration: number | null; marker?: string };
  const rows: Row[] = [
    ...scene.motion.flatMap<Row>(event => {
      const definition = catalog?.motions[event.play];
      if (event.show_parts && definition) return movementComponents(definition, trajectories[event.id]).map(part => ({
        track: "motion" as const, id: event.id, ...part, at: part.offset === null ? null : event.at + part.offset,
      }));
      return [{ track: "motion" as const, id: event.id, label: definition?.source === "draft" ? "Custom movement" : event.play, at: event.at, duration: trajectories[event.id]?.duration_seconds ?? null }];
    }),
    ...scene.lighting.map(event => ({ track: "lighting" as const, id: event.id, label: event.effect, at: triggerTime(event, scene, trajectories), duration: (event.duration ?? .8) + (event.transition ?? 0), marker: event.on_marker })),
    ...scene.audio.map(event => ({ track: "audio" as const, id: event.id, label: event.cue, at: triggerTime(event, scene, trajectories), duration: event.duration ?? null, marker: event.on_marker })),
  ];
  for (const track of ["lighting", "audio"] as const) for (const event of scene[track]) {
    if (!event.delay) continue;
    const at = triggerTime(event,scene,trajectories);
    rows.push({ track, id: event.id, label: "Delay", at: at === null ? null : at-event.delay, duration: event.delay, component: { index: 0, kind: "delay" } });
  }
  const rowKey = (row: TrackSelection) => `${row.track}:${row.id}:${row.component ? `${row.component.index}:${row.component.kind}` : "whole"}`;
  const isSelected = (row: TrackSelection) => !!selection && rowKey(row) === rowKey(selection);
  const selected = rows.find(isSelected);
  const pending = rows.filter(row => row.track !== "motion" && row.at === null);
  const resolved = rows.filter(row => !pending.includes(row));
  const scrub = (event: React.PointerEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    event.preventDefault(); event.stopPropagation();
    const element = event.currentTarget;
    const rect = element.parentElement!.getBoundingClientRect();
    element.setPointerCapture(event.pointerId);
    const update = (x: number) => onTimeChange(Math.max(0, Math.min(duration, (x - rect.left) / rect.width * duration)));
    update(event.clientX);
    element.onpointermove = move => update(move.clientX);
    const finish = () => { element.onpointermove = null; element.onpointerup = null; element.onpointercancel = null; };
    element.onpointerup = finish; element.onpointercancel = finish;
  };
  const playhead = () => <span role="slider" aria-label="Scene playhead" tabIndex={0} aria-valuemin={0} aria-valuemax={duration} aria-valuenow={currentTime} className="timeline-playhead" style={{ left: `${currentTime / duration * 100}%` }} onPointerDown={scrub} onKeyDown={event => { if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) { event.preventDefault(); onTimeChange(event.key === "Home" ? 0 : event.key === "End" ? duration : Math.max(0, Math.min(duration, currentTime + (event.key === "ArrowLeft" ? -.05 : .05)))); } }} />;
  const startDrag = (event: React.PointerEvent<HTMLButtonElement>, row: Row) => {
    if (event.button !== 0 || row.at === null || !onMove) return;
    event.stopPropagation();
    const element = event.currentTarget;
    const width = element.closest(".track-canvas")!.getBoundingClientRect().width;
    element.setPointerCapture(event.pointerId);
    drag.current = { x: event.clientX, time: row.at, target: row, width, element };
    element.onpointermove = move => {
      const active = drag.current; if (!active) return;
      element.style.transform = `translateX(${move.clientX - active.x}px)`;
      element.style.cursor = "grabbing";
    };
    element.onpointerup = up => {
      const active = drag.current;
      element.style.transform = ""; element.style.cursor = "";
      element.onpointermove = null; element.onpointerup = null; drag.current = null;
      if (!active || Math.abs(up.clientX - active.x) < 4) return;
      let time = Math.max(0, active.time + (up.clientX - active.x) / width * duration);
      const edges = [0, ...rows.filter(other => other.track === row.track && rowKey(other) !== rowKey(row)).flatMap(other => other.at === null ? [] : [other.at, other.at + (other.duration ?? 0)])];
      const nearest = edges.sort((a,b) => Math.abs(a-time)-Math.abs(b-time))[0];
      if (Math.abs(nearest-time) * width / duration <= 12) time = nearest;
      onMove(row, time);
    };
    element.onpointercancel = () => { drag.current = null; element.style.transform = ""; element.style.cursor = ""; element.onpointermove = null; element.onpointerup = null; };
  };
  const clip = (row: Row) => <button {...contextHandlers(row)} onPointerDown={event => startDrag(event, row)}
    aria-label={`${row.component?.kind ?? row.track}: ${row.label.replaceAll("_", " ")}, ${row.at === null ? "timing pending" : `${row.at.toFixed(2)} seconds`}`}
    aria-pressed={isSelected(row)}
    className={`track-clip ${row.at === null || row.duration === null && row.track === "motion" ? "component-pending" : ""} ${row.component?.kind ?? ""} ${row.track === "lighting" ? "light" : row.track} ${isSelected(row) ? "selected" : ""} ${row.track === "audio" ? "event-point" : ""}`}
    style={{ left: `${(row.at ?? 0) / duration * 100}%`, width: row.duration === null ? ".75rem" : `${row.duration / duration * 100}%` }}
    onClick={event => { event.stopPropagation(); onSelect({ track: row.track, id: row.id, component: row.component }); }}>
    {row.component?.kind === "delay" ? `Delay ${row.duration?.toFixed(2)} s` : row.label.replaceAll("_", " ")}
  </button>;
  const renderRow = (row: Row) => <div key={rowKey(row)} className="track-row" {...contextHandlers(row)}>
    <button className="track-event-label" aria-pressed={isSelected(row)} onClick={() => onSelect(row)}><small>{row.component?.kind ?? row.track}</small>{row.label.replaceAll("_", " ")}</button>
    <div className="track-canvas">{clip(row)}{playhead()}</div>
  </div>;
  return <section className="timeline" aria-label="Scene tracks">
    {menu && <div ref={menuContainer} className="movement-context-menu" role="menu" aria-label="Track item actions" style={{ left: menu.x, top: menu.y }} onKeyDown={event => {
      if (event.key === "Escape") setMenu(null);
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault(); const items = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>("button"));
        const index = items.indexOf(document.activeElement as HTMLButtonElement);
        items[(index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
      }
    }}>
      {menu.track === "motion" && !menu.component && !scene.motion.find(event => event.id === menu.id)?.show_parts && catalog && onChangeMovement && onToggleParts && <button role="menuitem" ref={menuRef} onClick={() => { revealSplit.current = menu.id; onToggleParts(menu.id); onSelect({ track: "motion", id: menu.id, component: { index: 0, kind: "pose" } }); setMenu(null); }}>Split into components</button>}
      {menu.track === "motion" && menu.component?.kind === "pose" && onEditPose && <button role="menuitem" ref={menuRef} onClick={() => { onEditPose(menu); setMenu(null); }}>Edit pose</button>}
      {onDelete && <button className="menu-delete" role="menuitem" ref={menu.track !== "motion" || !!menu.component || !onToggleParts ? menuRef : undefined} onClick={() => { onDelete({ track: menu.track, id: menu.id, component: menu.component }); setMenu(null); }}>Delete</button>}
    </div>}

    <header className="timeline-ruler"><strong>Scene timeline</strong><label>Zoom<select value={zoom} onChange={event => setZoom(Number(event.target.value))}><option value="1">Fit</option><option value="2">2×</option><option value="4">4×</option></select></label><span>{duration.toFixed(2)} s</span></header>
    <p className="timeline-selection">{selected ? `${selected.component?.kind ?? selected.track} · ${selected.label.replaceAll("_", " ")} · ${selected.at === null ? selected.marker ? `waiting for ${selected.marker}` : "timing pending" : `${selected.at.toFixed(2)} s`}` : "Select an event to edit its timing and settings."}</p>

    <div className="timeline-scroll"><div style={{ minWidth: `${zoom * 100}%` }}>
      {(["motion", "lighting", "audio"] as const).map(track => {
        const trackRows = resolved.filter(row => row.track === track);
        const title = { motion: "Motion", lighting: "Light", audio: "Sound" }[track];
        return <section key={track} className="motion-track-group" aria-label={`${title} track`}>
          <div className="track-row motion-group-header">
            <button className="track-event-label" ref={node => { if (track === "motion") for (const event of scene.motion) firstComponents.current[event.id] = node; }} aria-expanded={!!expanded[track]} aria-controls={`${track}-track-components`} onClick={() => setExpanded(value => ({ ...value, [track]: !value[track] }))}>{expanded[track] ? "▾" : "▸"} {title}<small>{trackRows.length} {trackRows.length === 1 ? "component" : "components"}</small></button>
            <div className={`track-canvas motion-summary ${trackRows.some(row => row.at === null || row.duration === null) ? "untimed" : ""}`}>{!expanded[track] && trackRows.map(row => <span key={rowKey(row)} className="motion-summary-clip">{clip(row)}</span>)}{playhead()}</div>
          </div>
          {expanded[track] && <div id={`${track}-track-components`} className="motion-group-children">{trackRows.map(renderRow)}</div>}
        </section>;
      })}
    </div></div>
    {!!pending.length && <section className="pending-events" aria-label="Events awaiting compilation"><strong>Calculating scene timing</strong><p>Movements play in order. Connect Orion to calculate their durations and place linked light and sound.</p>{pending.map(row => <button key={rowKey(row)} {...contextHandlers(row)} aria-pressed={isSelected(row)} onClick={() => onSelect({ track: row.track, id: row.id, component: row.component })}>{row.label.replaceAll("_", " ")} · {row.marker ? `linked to ${row.marker}` : row.track === "motion" ? "timing set automatically" : `starts at ${row.at?.toFixed(2)} s`}</button>)}</section>}
  </section>;
}
