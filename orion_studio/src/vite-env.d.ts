/// <reference types="vite/client" />

declare module "*.yaml?raw" {
  const contents: string;
  export default contents;
}

declare module "*.urdf?raw" {
  const contents: string;
  export default contents;
}

declare module "*.stl?url" {
  const url: string;
  export default url;
}

declare module "urdf-loader" {
  import { LoadingManager, Material, Object3D } from "three";

  export default class URDFLoader {
    manager: LoadingManager;
    packages: string | Record<string, string>;
    loadMeshCb: (
      path: string,
      manager: LoadingManager,
      material: Material,
      done: (mesh: Object3D | null, error?: Error) => void,
    ) => void;
    parse(contents: string): URDFRobot;
  }

  export interface URDFJoint extends Object3D {
    setJointValue(value: number): void;
  }

  export interface URDFRobot extends Object3D {
    joints: Record<string, URDFJoint>;
  }
}
