import { eventDuration } from "./preview";
import type { ProjectCatalog, SceneDefinition, SceneEvent } from "../types";

export type TimelineLane = "movement" | "light" | "audio";

export interface DuplicatedTimelineSelection {
  scene: SceneDefinition;
  duplicatedIds: string[];
  startsAt: number;
}

export function timelineLane(event: SceneEvent): TimelineLane {
  if (event.type === "light") return "light";
  if (event.type === "audio") return "audio";
  return "movement";
}

export function duplicateTimelineEvents(
  scene: SceneDefinition,
  eventIds: string[],
  catalog: ProjectCatalog,
  makeId: () => string = () => crypto.randomUUID(),
  durationOf: (event: SceneEvent) => number = (event) => eventDuration(event, catalog),
): DuplicatedTimelineSelection {
  const requested = new Set(eventIds);
  const selected = scene.timeline
    .filter((event) => requested.has(event.id))
    .sort((left, right) => left.at - right.at);
  if (selected.length === 0) {
    return { scene, duplicatedIds: [], startsAt: 0 };
  }

  const lanes = new Set(selected.map(timelineLane));
  if (lanes.size !== 1) {
    throw new Error("Select clips from one track before duplicating them together.");
  }

  const selectionStart = selected[0].at;
  const selectionEnd = Math.max(
    ...selected.map((event) => event.at + durationOf(event)),
  );
  const draftPoses = { ...(scene.draftPoses ?? {}) };
  const copiedDraftNames = new Map<string, string>();
  const duplicated = selected.map((event) => {
    const copy = structuredClone(event);
    copy.id = `copy:${event.id}:${makeId()}`;
    copy.at = Number((selectionEnd + event.at - selectionStart).toFixed(3));

    if (copy.type === "goto_pose" && draftPoses[copy.pose]) {
      let copiedName = copiedDraftNames.get(copy.pose);
      if (!copiedName) {
        copiedName = `studio_draft_${makeId().replaceAll("-", "_")}`;
        copiedDraftNames.set(copy.pose, copiedName);
        draftPoses[copiedName] = {
          ...structuredClone(draftPoses[copy.pose]),
          name: copiedName,
        };
      }
      copy.pose = copiedName;
    }
    return copy;
  });

  return {
    scene: {
      ...scene,
      source: "draft",
      draftPoses,
      timeline: [...scene.timeline, ...duplicated].sort((left, right) => left.at - right.at),
    },
    duplicatedIds: duplicated.map((event) => event.id),
    startsAt: duplicated[0].at,
  };
}
