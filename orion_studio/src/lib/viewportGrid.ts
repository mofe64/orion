import { Box3, Matrix4, Vector3 } from "three";

/** Align the display grid to the stationary CAD base, after URDF and Z-up → Y-up transforms. */
export function baseGridFrame(bounds: Box3, meshToWorld: Matrix4): { center: Vector3; yaw: number } {
  const center = bounds.getCenter(new Vector3()).applyMatrix4(meshToWorld);
  const edge = new Vector3(1, 0, 0).transformDirection(meshToWorld);
  // GridHelper lies in XZ. A positive Three.js Y rotation points its X axis toward -Z.
  return { center: center.setY(0), yaw: Math.atan2(-edge.z, edge.x) };
}
