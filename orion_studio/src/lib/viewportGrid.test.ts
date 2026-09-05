/// <reference types="node" />
import { readFileSync } from "node:fs";
import { Box3, Euler, Matrix4, Quaternion, Vector3 } from "three";
import { STLLoader } from "three/examples/jsm/loaders/STLLoader.js";
import { describe, expect, it } from "vitest";
import { baseGridFrame } from "./viewportGrid";

const bounds = new Box3(new Vector3(-1, -2, 0), new Vector3(1, 2, 1));

describe("Create floor grid alignment", () => {
  it("follows a rotated, offset stationary base without modifying the model transform", () => {
    const matrix = new Matrix4().compose(new Vector3(.3, .1, -.2), new Quaternion().setFromEuler(new Euler(-Math.PI / 2, 0, 0)), new Vector3(1, 1, 1));
    matrix.premultiply(new Matrix4().makeRotationY(.6));
    const before = matrix.clone();
    const frame = baseGridFrame(bounds, matrix);
    expect(frame.yaw).toBeCloseTo(.6);
    expect(frame.center.y).toBe(0);
    const actualCenter = bounds.getCenter(new Vector3()).applyMatrix4(matrix);
    expect(frame.center.x).toBeCloseTo(actualCenter.x);
    expect(frame.center.z).toBeCloseTo(actualCenter.z);
    expect(matrix.equals(before)).toBe(true);
  });

  it("centers and aligns the shipped Orion base's edges with the grid", () => {
    const urdf = readFileSync(new URL("../../../description/urdf/orion.urdf", import.meta.url), "utf8");
    const base = urdf.match(/<link name="base_link">([\s\S]*?)<\/link>/)![1];
    const visual = base.match(/<visual>([\s\S]*?)<\/visual>/)![1];
    const position = visual.match(/xyz="([^"]+)"/)![1].split(" ").map(Number);
    const rotation = visual.match(/rpy="([^"]+)"/)![1].split(" ").map(Number);
    const source = readFileSync(new URL("../../../description/meshes/lamp_base.stl", import.meta.url));
    const geometry = new STLLoader().parse(source.buffer.slice(source.byteOffset, source.byteOffset + source.byteLength) as ArrayBuffer);
    geometry.computeBoundingBox();
    const cadTransform = new Matrix4().compose(new Vector3(...position), new Quaternion().setFromEuler(new Euler(rotation[0], rotation[1], rotation[2], "ZYX")), new Vector3(1, 1, 1));
    const world = new Matrix4().makeRotationX(-Math.PI / 2).multiply(new Matrix4().makeTranslation(0, 0, .0418)).multiply(cadTransform);
    const frame = baseGridFrame(geometry.boundingBox!, world);
    expect(frame.center.x).toBeCloseTo(0, 5);
    expect(frame.center.z).toBeCloseTo(0, 5);
    const gridRotation = new Matrix4().makeRotationY(frame.yaw);
    const gridX = new Vector3(1, 0, 0).transformDirection(gridRotation);
    const gridZ = new Vector3(0, 0, 1).transformDirection(gridRotation);
    expect(Math.abs(gridX.dot(new Vector3(1, 0, 0).transformDirection(world)))).toBeCloseTo(1);
    expect(Math.abs(gridZ.dot(new Vector3(0, 1, 0).transformDirection(world)))).toBeCloseTo(1);
    geometry.dispose();
  });
});
