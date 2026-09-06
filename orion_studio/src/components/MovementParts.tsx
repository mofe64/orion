import type { MotionDefinition, ProjectCatalog } from "../types";

export function updateMovementPart(motion: MotionDefinition, index: number, changes: Partial<MotionDefinition["keyframes"][number]>): MotionDefinition {
  return { ...motion, keyframes: motion.keyframes.map((frame, i) => i === index ? { ...frame, ...changes } : frame) };
}

export function MovementParts({ motion, catalog, onChange }: { motion: MotionDefinition; catalog: ProjectCatalog; onChange: (motion: MotionDefinition) => void }) {
  const patch = (index: number, changes: Partial<MotionDefinition["keyframes"][number]>) => onChange(updateMovementPart(motion, index, changes));
  return <div className="movement-parts" aria-label="Poses and delays">
    {motion.keyframes.map((frame, index) => <div className="movement-part" key={index}>
      <span className="part-number">{index + 1}</span>
      {frame.pose ? <label>Pose<select value={frame.pose} onChange={event => patch(index, { pose: event.target.value })}>{Object.keys(catalog.poses).map(name => <option key={name} value={name}>{name.replaceAll("_", " ")}</option>)}</select></label> : <span>Relative movement {index + 1}</span>}
      <label>Move duration (s)<input type="number" min={0.02} step={0.05} value={frame.duration} onChange={event => { const duration = Number(event.target.value); if (Number.isFinite(duration) && duration >= .02) patch(index, { duration }); }} /></label>
      {frame.arrival === "settle" ? <label>Pause after (s)<input type="number" min={0} step={0.05} value={frame.hold} onChange={event => { const hold = Number(event.target.value); if (Number.isFinite(hold) && hold >= 0) patch(index, { hold }); }} /></label> : <span className="part-flow">Continues smoothly</span>}
    </div>)}
    <p>Orion adjusts movement time when needed. Smooth transitions stay connected.</p>
  </div>;
}
