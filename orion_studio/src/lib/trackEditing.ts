import type { ProjectCatalog, SceneDefinition } from "../types";
import type { TrackSelection } from "../components/Timeline";
import { triggerTime, type SceneTrajectoryPreviews } from "./preview";

/** Channels run independently, but each channel has a single owner at a time. */
export function sequenceMedia(scene: SceneDefinition, trajectories: SceneTrajectoryPreviews): SceneDefinition {
  let changed = false;
  const sequence = <T extends SceneDefinition["lighting"][number] | SceneDefinition["audio"][number]>(events: T[]) => {
    let end = 0;
    return events.map(event => {
      const requested = triggerTime({ ...event, resolved_at: undefined }, scene, trajectories);
      if (requested === null && !event.after_previous) {
        if (event.resolved_at !== undefined) { changed = true; return { ...event, resolved_at: undefined }; }
        return event;
      }
      const at = Math.max(event.after_previous ? end : requested ?? 0, end) + (event.delay ?? 0);
      end = at + (event.duration ?? .8) + ("transition" in event ? event.transition ?? 0 : 0);
      // Preserve marker intent; resolved_at is not serialized. Materialize only in prepared scene.
      if (event.resolved_at === at) return event;
      changed = true;
      return { ...event, resolved_at: at };
    });
  };
  const lighting = sequence(scene.lighting), audio = sequence(scene.audio);
  return changed ? { ...scene, lighting, audio } : scene;
}

export function moveTrackEvent(scene: SceneDefinition, target: TrackSelection, time: number, trajectories: SceneTrajectoryPreviews): SceneDefinition {
  if (target.track === "motion") {
    const ordered = scene.motion.map(event => event.id === target.id ? { ...event, at: Math.max(0,time), after_previous: false } : event).sort((a,b) => a.at-b.at || (a.id === target.id ? -1 : b.id === target.id ? 1 : 0));
    let end = 0;
    return { ...scene, motion: ordered.map(event => {
      const at = Math.max(end,event.at); end = at + (trajectories[event.id]?.duration_seconds ?? 0);
      return { ...event, at, after_previous: false };
    }) };
  }
  const items = scene[target.track].map(event => event.id === target.id ? { ...event, at: time, resolved_at: undefined, on_marker: undefined, after_previous: undefined } : event).sort((a,b) => (triggerTime(a,scene,trajectories) ?? 0)-(triggerTime(b,scene,trajectories) ?? 0) || (a.id === target.id ? -1 : b.id === target.id ? 1 : 0));
  return sequenceMedia({ ...scene, [target.track]: items },trajectories);
}

const audioLengths = new Map<string, Promise<number>>();
export async function withSoundDurations(scene: SceneDefinition, catalog: ProjectCatalog): Promise<SceneDefinition> {
  const audio = await Promise.all(scene.audio.map(async event => {
    const url = catalog.cueUrls[event.cue];
    if (!url) throw new Error(`Sound ${event.cue.replaceAll("_", " ")} is unavailable.`);
    if (!audioLengths.has(url)) audioLengths.set(url, fetch(url).then(async response => {
      if (!response.ok) throw new Error("Could not load sound timing.");
      const bytes = await response.arrayBuffer(), view = new DataView(bytes);
      let rate = 0, size = 0;
      for (let offset = 12; offset + 8 <= bytes.byteLength;) {
        const id = String.fromCharCode(...new Uint8Array(bytes,offset,4));
        const length = view.getUint32(offset+4,true);
        if (id === "fmt ") rate = view.getUint32(offset+16,true);
        if (id === "data") size += length;
        offset += 8 + length + (length % 2);
      }
      if (!rate || !size) throw new Error("Could not read sound duration.");
      return size/rate;
    }).catch(error => { audioLengths.delete(url); throw error; }));
    return { ...event, duration: await audioLengths.get(url)! };
  }));
  return { ...scene, audio };
}

export function validateMediaStarts(scene: SceneDefinition, trajectories: SceneTrajectoryPreviews) {
  for (const event of [...scene.lighting,...scene.audio]) {
    if (triggerTime(event,scene,trajectories) === null) throw new Error(`Choose a start for ${("effect" in event ? event.effect : event.cue).replaceAll("_", " ")}. Its movement cue ${event.on_marker?.replaceAll("_", " ")} is no longer available.`);
  }
}
