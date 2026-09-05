import { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { STLLoader } from "three/examples/jsm/loaders/STLLoader.js";
import URDFLoader, { type URDFRobot } from "urdf-loader";

import type { JointPositions, LightPreview, ProjectCatalog } from "../types";

interface RobotViewportProps {
  catalog: ProjectCatalog;
  joints: JointPositions;
  light: LightPreview;
}

function previewColor(light: LightPreview): THREE.Color {
  const warm = light.white / 255;
  return new THREE.Color(
    Math.min(1, light.red / 255 + warm),
    Math.min(1, light.green / 255 + warm * 0.72),
    Math.min(1, light.blue / 255 + warm * 0.42),
  );
}

let savedCamera: { position: THREE.Vector3; target: THREE.Vector3 } | null = null;

export function RobotViewport({ catalog, joints, light }: RobotViewportProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const robotRef = useRef<URDFRobot | null>(null);
  const lampLightRef = useRef<THREE.PointLight | null>(null);
  const invalidateRef = useRef<() => void>(() => {});
  const [renderError, setRenderError] = useState<string | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    let disposed = false;
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x0a0f17);
    scene.fog = new THREE.FogExp2(0x0a0f17, 0.55);

    const camera = new THREE.PerspectiveCamera(38, 1, 0.01, 50);
    camera.position.set(0.78, 0.62, 0.72);

    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
      setRenderError(null);
    } catch {
      setRenderError("3D preview is unavailable because this system could not create a WebGL context.");
      return;
    }
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.type = THREE.PCFShadowMap;
    host.append(renderer.domElement);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.target.set(0, 0.24, 0);
    if (savedCamera) { camera.position.copy(savedCamera.position); controls.target.copy(savedCamera.target); }
    controls.enableDamping = false;
    controls.minDistance = 0.35;
    controls.maxDistance = 2.5;
    controls.update();

    scene.add(new THREE.HemisphereLight(0xb9d6ff, 0x18202a, 2.1));
    const key = new THREE.DirectionalLight(0xffffff, 3.2);
    key.position.set(1.2, 1.5, 0.8);
    key.castShadow = true;
    scene.add(key);

    const lampLight = new THREE.PointLight(0xffc56d, 0, 1.6, 1.5);
    lampLight.position.set(0, 0.56, 0.12);
    scene.add(lampLight);
    lampLightRef.current = lampLight;

    const floor = new THREE.Mesh(
      new THREE.CircleGeometry(0.62, 64),
      new THREE.MeshStandardMaterial({ color: 0x111a26, roughness: 0.86, metalness: 0.05 }),
    );
    floor.rotation.x = -Math.PI / 2;
    floor.receiveShadow = true;
    scene.add(floor);

    const grid = new THREE.GridHelper(1.3, 20, 0x34445b, 0x1d2938);
    grid.position.y = 0.001;
    scene.add(grid);

    const manager = new THREE.LoadingManager();
    const urdfLoader = new URDFLoader();
    urdfLoader.manager = manager;
    urdfLoader.loadMeshCb = (path, loadingManager, _material, done) => {
      const name = path.split("/").at(-1) ?? path;
      const url = catalog.meshUrls[name];
      if (!url) {
        done(null, new Error(`Missing Orion mesh: ${name}`));
        return;
      }
      new STLLoader(loadingManager).load(
        url,
        (geometry) => {
          if (disposed) { geometry.dispose(); return; }
          geometry.computeVertexNormals();
          const mesh = new THREE.Mesh(
            geometry,
            new THREE.MeshStandardMaterial({
              color: name.includes("lamphead") || name.includes("diffuser") ? 0xe7e2d6 : 0x596474,
              roughness: 0.62,
              metalness: 0.08,
            }),
          );
          mesh.castShadow = true;
          mesh.receiveShadow = true;
          done(mesh);
          invalidateRef.current();
        },
        undefined,
        (error) => done(null, error instanceof Error ? error : new Error(String(error))),
      );
    };

    const robot = urdfLoader.parse(catalog.urdf);
    robot.rotation.x = -Math.PI / 2;
    scene.add(robot);
    robotRef.current = robot;

    const resize = () => {
      const width = Math.max(1, host.clientWidth);
      const height = Math.max(1, host.clientHeight);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      renderer.setSize(width, height, false);
      invalidateRef.current();
    };
    const observer = new ResizeObserver(resize);
    observer.observe(host);
    resize();

    let frame = 0;
    const invalidate = () => {
      if (disposed || frame || document.hidden) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        if (!disposed && !document.hidden) renderer.render(scene, camera);
      });
    };
    invalidateRef.current = invalidate;
    controls.addEventListener("change", invalidate);
    document.addEventListener("visibilitychange", invalidate);
    invalidate();

    return () => {
      savedCamera = { position: camera.position.clone(), target: controls.target.clone() };
      disposed = true;
      invalidateRef.current = () => {};
      cancelAnimationFrame(frame);
      document.removeEventListener("visibilitychange", invalidate);
      controls.removeEventListener("change", invalidate);
      scene.traverse((object) => {
        const mesh = object as THREE.Mesh;
        mesh.geometry?.dispose();
        const materials = mesh.material ? (Array.isArray(mesh.material) ? mesh.material : [mesh.material]) : [];
        materials.forEach((material) => material.dispose());
      });
      observer.disconnect();
      controls.dispose();
      key.shadow.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      renderer.domElement.remove();
      robotRef.current = null;
      lampLightRef.current = null;
    };
  }, [catalog]);

  useEffect(() => {
    const robot = robotRef.current;
    if (!robot) return;
    for (const [name, value] of Object.entries(joints)) {
      robot.joints[name]?.setJointValue(value + catalog.urdfJointOffsets[name as keyof JointPositions]);
    }
    invalidateRef.current();
  }, [catalog.urdfJointOffsets, joints]);

  useEffect(() => {
    const lamp = lampLightRef.current;
    if (!lamp) return;
    lamp.color.copy(previewColor(light));
    lamp.intensity = Math.max(light.red, light.green, light.blue, light.white) / 22;
    invalidateRef.current();
  }, [light]);

  return (
    <div className="robot-viewport" ref={hostRef} aria-label="Interactive 3D preview of Orion">
      <div className="viewport-badge">3D model</div>
      {renderError ? (
        <div className="viewport-fallback" role="status">
          <span>Preview unavailable</span>
          <p>{renderError}</p>
        </div>
      ) : (
        <div className="viewport-help">Drag to orbit · scroll to zoom</div>
      )}
    </div>
  );
}
