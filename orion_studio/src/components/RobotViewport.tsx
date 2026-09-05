import { useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { STLLoader } from "three/examples/jsm/loaders/STLLoader.js";
import URDFLoader, { type URDFRobot } from "urdf-loader";

import type { JointPositions, LightPreview, ProjectCatalog } from "../types";

interface RobotViewportProps {
  catalog: ProjectCatalog;
  joints: JointPositions;
  light: LightPreview;
  mode?: "editor" | "home";
  theme?: "dark" | "light";
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

export function RobotViewport({ catalog, joints, light, mode = "editor", theme = "dark" }: RobotViewportProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const robotRef = useRef<URDFRobot | null>(null);
  const invalidateRef = useRef<() => void>(() => {});
  const rotateRef = useRef<(angle: number) => void>(() => {});
  const themeUpdateRef = useRef<() => void>(() => {});
  const lightUpdateRef = useRef<() => void>(() => {});
  const themeRef = useRef(theme);
  const lightRef = useRef(light);
  themeRef.current = theme;
  lightRef.current = light;
  const [renderError, setRenderError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    let disposed = false;
    setLoading(true);
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
    if (mode === "editor" && savedCamera) { camera.position.copy(savedCamera.position); controls.target.copy(savedCamera.target); }
    controls.enableDamping = false;
    controls.minDistance = 0.35;
    controls.maxDistance = 2.5;
    if (mode === "home") {
      controls.enableZoom = false;
      controls.enablePan = false;
      controls.minDistance = controls.maxDistance = camera.position.distanceTo(controls.target);
      controls.minPolarAngle = .35;
      controls.maxPolarAngle = 1.5;
    }
    controls.update();
    rotateRef.current = angle => {
      camera.position.sub(controls.target).applyAxisAngle(new THREE.Vector3(0, 1, 0), angle).add(controls.target);
      controls.update();
    };
    renderer.domElement.setAttribute("aria-label", mode === "home" ? "Orion 3D model. Drag to rotate; zoom is disabled." : "Orion 3D model");
    renderer.domElement.setAttribute("role", "img");

    scene.add(new THREE.HemisphereLight(0xb9d6ff, 0x18202a, 2.1));
    const key = new THREE.DirectionalLight(0xffffff, 3.2);
    key.position.set(1.2, 1.5, 0.8);
    key.castShadow = true;
    scene.add(key);

    const lampLight = new THREE.PointLight(0xffc56d, 0, 1.6, 1.5);
    lampLight.position.set(0, 0.56, 0.12);
    scene.add(lampLight);

    const floor = new THREE.Mesh(
      new THREE.CircleGeometry(mode === "home" ? 0.39 : 0.62, 64),
      new THREE.MeshStandardMaterial({ color: 0x111a26, roughness: 0.86, metalness: 0.05 }),
    );
    floor.rotation.x = -Math.PI / 2;
    floor.receiveShadow = true;
    scene.add(floor);

    const grid = new THREE.GridHelper(1.3, 20, 0x34445b, 0x1d2938);
    grid.position.y = 0.001;
    if (mode === "editor") scene.add(grid);
    else { grid.geometry.dispose(); (grid.material as THREE.Material).dispose(); }

    themeUpdateRef.current = () => {
      const pale = mode === "home" && themeRef.current === "light";
      scene.background = new THREE.Color(pale ? 0xf0f3f8 : 0x0a0f17);
      scene.fog = mode === "home" ? null : new THREE.FogExp2(0x0a0f17, .55);
      floor.material.color.set(pale ? 0xe2e8f0 : 0x111a26);
      invalidateRef.current();
    };
    themeUpdateRef.current();

    const diffusers: THREE.MeshStandardMaterial[] = [];
    lightUpdateRef.current = () => {
      const current = lightRef.current;
      const color = previewColor(current);
      lampLight.color.copy(color);
      lampLight.intensity = Math.max(current.red, current.green, current.blue, current.white) / 22;
      for (const material of diffusers) {
        material.emissive.copy(color);
        material.emissiveIntensity = Math.max(current.red, current.green, current.blue, current.white) / 255 * 1.8;
      }
      invalidateRef.current();
    };

    const manager = new THREE.LoadingManager();
    manager.onError = () => { if (!disposed) setRenderError("Orion's model could not be loaded. Reopen this screen to retry."); };
    // Three coalesces in-flight URLs across loaders (including StrictMode's remount).
    // Count this view's callbacks; a shared request may finish on another manager.
    let remainingMeshes = 0;
    const meshComplete = () => {
      remainingMeshes -= 1;
      if (!disposed && remainingMeshes === 0) setLoading(false);
    };
    const urdfLoader = new URDFLoader();
    urdfLoader.manager = manager;
    urdfLoader.loadMeshCb = (path, loadingManager, _material, done) => {
      remainingMeshes += 1;
      const name = path.split("/").at(-1) ?? path;
      const url = catalog.meshUrls[name];
      if (!url) {
        if (!disposed) setRenderError(`Missing Orion mesh: ${name}`);
        done(null, new Error(`Missing Orion mesh: ${name}`));
        meshComplete();
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
          if (mode === "home" && name.includes("diffuser")) {
            diffusers.push(mesh.material);
            lightUpdateRef.current();
          }
          done(mesh);
          meshComplete();
          invalidateRef.current();
        },
        undefined,
        (error) => {
          if (!disposed) setRenderError("Orion's model could not be loaded. Reopen this screen to retry.");
          done(null, error instanceof Error ? error : new Error(String(error)));
          meshComplete();
        },
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
      if (mode === "editor") savedCamera = { position: camera.position.clone(), target: controls.target.clone() };
      disposed = true;
      invalidateRef.current = () => {};
      rotateRef.current = () => {};
      themeUpdateRef.current = () => {};
      lightUpdateRef.current = () => {};
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
    };
  }, [catalog, mode]);

  useEffect(() => {
    const robot = robotRef.current;
    if (!robot) return;
    for (const [name, value] of Object.entries(joints)) {
      robot.joints[name]?.setJointValue(value + catalog.urdfJointOffsets[name as keyof JointPositions]);
    }
    invalidateRef.current();
  }, [catalog, joints, mode]);

  useEffect(() => {
    lightUpdateRef.current();
  }, [catalog, light, mode]);

  useEffect(() => { themeUpdateRef.current(); }, [theme]);

  return (
    <div className="robot-viewport" ref={hostRef} aria-label="Interactive 3D preview of Orion">
      {mode === "editor" && <div className="viewport-badge">3D model</div>}
      {renderError ? (
        <div className="viewport-fallback" role="status">
          <span>Preview unavailable</span>
          <p>{renderError}</p>
        </div>
      ) : mode === "home" ? (
        <>
          {loading && <p className="viewport-loading" role="status">Loading Orion’s 3D model…</p>}
          <div className="viewport-orbit-controls">
            <button aria-label="Rotate model left" disabled={loading} onClick={() => rotateRef.current(-.22)}><ChevronLeft size={16} /></button>
            <span>Drag to rotate · Fixed zoom</span>
            <button aria-label="Rotate model right" disabled={loading} onClick={() => rotateRef.current(.22)}><ChevronRight size={16} /></button>
          </div>
        </>
      ) : (
        <div className="viewport-help">Drag to orbit · scroll to zoom</div>
      )}
    </div>
  );
}
