import { invoke } from "@tauri-apps/api/core";
import { load as loadYaml } from "js-yaml";
import {
  CircleStop,
  CloudCog,
  Lightbulb,
  Link2,
  Move3d,
  Music2,
  Plus,
  Radio,
  Save,
  Sparkles,
  Unplug,
  Volume2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { EventInspector } from "./components/EventInspector";
import { RobotViewport } from "./components/RobotViewport";
import { Timeline } from "./components/Timeline";
import { projectCatalog } from "./lib/catalog";
import {
  cancelRun,
  getCapabilities,
  getUserScene,
  getStatus,
  gotoPose,
  listUserScenes,
  publishScene,
  runMotion,
  runScene,
  updateUserScene,
  type GatewayConnection,
  type UserSceneSource,
} from "./lib/gateway";
import { sampleSceneLight, sampleScenePose, sceneDuration } from "./lib/preview";
import type {
  GatewayCapabilities,
  GatewayStatus,
  SceneDefinition,
  SceneEvent,
  StoredSceneDocument,
} from "./types";

type PreviewKind = "scene" | "pose" | "motion";

function copyScene(scene: SceneDefinition): SceneDefinition {
  return structuredClone(scene);
}

function sortedTimeline(events: SceneEvent[]): SceneEvent[] {
  return [...events].sort((left, right) => left.at - right.at);
}

function sceneDocument(scene: SceneDefinition, name: string) {
  return {
    format_version: 1,
    scene: {
      name,
      description: scene.description,
      timeline: scene.timeline.map(({ id: _id, ...event }) => event),
    },
  };
}

function storedScene(document: StoredSceneDocument, remoteRevision?: string): SceneDefinition {
  return {
    format_version: 1,
    name: document.scene.name,
    description: document.scene.description,
    source: "user",
    remote_revision: remoteRevision,
    timeline: document.scene.timeline.map((event) => ({
      ...event,
      id: crypto.randomUUID(),
    })) as SceneEvent[],
  };
}

function remoteScene(source: UserSceneSource): SceneDefinition {
  const document = loadYaml(source.yaml) as StoredSceneDocument;
  if (
    !document
    || document.format_version !== 1
    || !document.scene
    || document.scene.name !== source.name
    || !Array.isArray(document.scene.timeline)
  ) {
    throw new Error(`Pi user scene '${source.name}' does not match its library metadata.`);
  }
  return storedScene(document, source.revision);
}

function createEvent(type: SceneEvent["type"], at: number, catalog = projectCatalog): SceneEvent {
  const id = crypto.randomUUID();
  switch (type) {
    case "play_motion":
      return { id, at, type, motion: Object.keys(projectCatalog.motions)[0] };
    case "goto_pose":
      return { id, at, type, pose: Object.keys(projectCatalog.poses)[0], duration_seconds: 2 };
    case "light":
      return { id, at, type, red: 8, green: 3, blue: 0, white: 20, transition_seconds: 0.35 };
    case "audio":
      return { id, at, type, cue: projectCatalog.cues[0] ?? "acknowledge" };
  }
}

function runLabel(run: { state: string; name?: string; run_id: number } | null | undefined): string {
  if (!run) return "No active run";
  return `${run.name ?? "run"} · ${run.state} · #${run.run_id}`;
}

export default function App() {
  const firstScene = projectCatalog.scenes.acknowledge_left ?? Object.values(projectCatalog.scenes)[0];
  const [sceneCatalog, setSceneCatalog] = useState(projectCatalog.scenes);
  const [scene, setScene] = useState<SceneDefinition>(() => copyScene(firstScene));
  const [previewKind, setPreviewKind] = useState<PreviewKind>("scene");
  const [selectedLibraryScene, setSelectedLibraryScene] = useState(firstScene.name);
  const [selectedEventId, setSelectedEventId] = useState<string | null>(scene.timeline[0]?.id ?? null);
  const [currentTime, setCurrentTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [saveAsName, setSaveAsName] = useState(`${firstScene.name}_studio`);
  const [projectRoot, setProjectRoot] = useState("");
  const [notice, setNotice] = useState("Built-in assets loaded from the Orion project.");

  const [connectionOpen, setConnectionOpen] = useState(false);
  const [gatewayUrl, setGatewayUrl] = useState(() => localStorage.getItem("orionStudioGateway") ?? "http://orion.local:7447");
  const [gatewayToken, setGatewayToken] = useState(() => sessionStorage.getItem("orionStudioToken") ?? "");
  const [connected, setConnected] = useState(false);
  const [robotStatus, setRobotStatus] = useState<GatewayStatus | null>(null);
  const [robotCapabilities, setRobotCapabilities] = useState<GatewayCapabilities | null>(null);
  const [cueDurations, setCueDurations] = useState<Record<string, number>>({});
  const [previewCue, setPreviewCue] = useState<string | null>(null);
  const playbackFrame = useRef(0);
  const previewAudio = useRef<HTMLAudioElement | null>(null);
  const previewAudioQueue = useRef<string[]>([]);
  const dispatchedPreviewCues = useRef(new Set<string>());
  const catalog = useMemo(
    () => ({ ...projectCatalog, scenes: sceneCatalog }),
    [sceneCatalog],
  );

  const duration = useMemo(() => {
    const cueEnd = scene.timeline.reduce((latest, event) => {
      if (event.type !== "audio") return latest;
      return Math.max(latest, event.at + (cueDurations[event.cue] ?? 1));
    }, 0);
    return Math.max(sceneDuration(scene, catalog), cueEnd);
  }, [scene, cueDurations, catalog]);
  const previewJoints = useMemo(
    () => sampleScenePose(scene, catalog, currentTime),
    [scene, catalog, currentTime],
  );
  const previewLight = useMemo(() => sampleSceneLight(scene, currentTime), [scene, currentTime]);
  const selectedEvent = scene.timeline.find((event) => event.id === selectedEventId) ?? null;
  const connection: GatewayConnection = { url: gatewayUrl, token: gatewayToken };
  const activeRobotRun = robotStatus?.scene.active ?? robotStatus?.runtime.motion;
  const latestRobotRun = activeRobotRun ?? robotStatus?.scene.last ?? robotStatus?.runtime.last_motion;
  const robotRunSummary = activeRobotRun
    ? `Active: ${runLabel(activeRobotRun)}`
    : latestRobotRun
      ? `Last: ${runLabel(latestRobotRun)}`
      : "No runs yet";

  const stopPreviewAudio = useCallback(() => {
    previewAudioQueue.current = [];
    if (previewAudio.current) {
      previewAudio.current.pause();
      previewAudio.current.currentTime = 0;
      previewAudio.current = null;
    }
    setPreviewCue(null);
  }, []);

  const playNextPreviewCue = useCallback(function playNextPreviewCue() {
    if (previewAudio.current) return;
    const cue = previewAudioQueue.current.shift();
    if (!cue) return;
    const url = catalog.cueUrls[cue];
    if (!url) return playNextPreviewCue();
    const audio = new Audio(url);
    previewAudio.current = audio;
    setPreviewCue(cue);
    const finished = () => {
      if (previewAudio.current === audio) previewAudio.current = null;
      setPreviewCue(null);
      playNextPreviewCue();
    };
    audio.addEventListener("ended", finished, { once: true });
    audio.addEventListener("error", finished, { once: true });
    void audio.play().catch(finished);
  }, [catalog.cueUrls]);

  const resetPreviewCues = (time: number) => {
    stopPreviewAudio();
    dispatchedPreviewCues.current.clear();
    for (const event of scene.timeline) {
      if (event.type === "audio" && event.at < time) {
        dispatchedPreviewCues.current.add(event.id);
      }
    }
  };

  useEffect(() => {
    invoke<string>("default_project_root")
      .then((root) => {
        setProjectRoot(root);
        invoke<StoredSceneDocument[]>("load_user_scenes", { projectRoot: root })
          .then((documents) => {
            setSceneCatalog((current) => {
              const next = { ...current };
              for (const document of documents) {
                if (next[document.scene.name]?.source === "built_in") continue;
                next[document.scene.name] = storedScene(document);
              }
              return next;
            });
          })
          .catch((error) => setNotice(`Could not load user scenes: ${String(error)}`));
      })
      .catch(() => setProjectRoot("Desktop save is available inside the Tauri shell."));
  }, []);

  useEffect(() => {
    const audioElements: HTMLAudioElement[] = [];
    for (const [cue, url] of Object.entries(catalog.cueUrls)) {
      const audio = new Audio(url);
      const loaded = () => {
        if (Number.isFinite(audio.duration)) {
          setCueDurations((current) => ({ ...current, [cue]: audio.duration }));
        }
      };
      audio.addEventListener("loadedmetadata", loaded);
      audioElements.push(audio);
    }
    return () => {
      for (const audio of audioElements) audio.src = "";
    };
  }, [catalog.cueUrls]);

  useEffect(() => {
    cancelAnimationFrame(playbackFrame.current);
    if (!playing) return;
    const startedAt = performance.now() - currentTime * 1000;
    const update = (now: number) => {
      const next = (now - startedAt) / 1000;
      if (next >= duration) {
        setCurrentTime(duration);
        setPlaying(false);
        return;
      }
      setCurrentTime(next);
      playbackFrame.current = requestAnimationFrame(update);
    };
    playbackFrame.current = requestAnimationFrame(update);
    return () => cancelAnimationFrame(playbackFrame.current);
  }, [playing, duration]);

  useEffect(() => {
    if (!playing) return;
    for (const event of scene.timeline) {
      if (
        event.type === "audio"
        && event.at <= currentTime
        && !dispatchedPreviewCues.current.has(event.id)
      ) {
        dispatchedPreviewCues.current.add(event.id);
        previewAudioQueue.current.push(event.cue);
      }
    }
    playNextPreviewCue();
  }, [currentTime, playing, scene.timeline, playNextPreviewCue]);

  useEffect(() => {
    if (!playing) stopPreviewAudio();
  }, [playing, stopPreviewAudio]);

  useEffect(() => {
    stopPreviewAudio();
    dispatchedPreviewCues.current.clear();
  }, [scene.name, stopPreviewAudio]);

  useEffect(() => {
    if (!connected) return;
    const poll = window.setInterval(() => {
      getStatus(connection)
        .then(setRobotStatus)
        .catch((error) => {
          setConnected(false);
          setNotice(`Robot connection lost: ${error.message}`);
        });
    }, 750);
    return () => window.clearInterval(poll);
  }, [connected, gatewayUrl, gatewayToken]);

  const loadScene = (name: string) => {
    resetPreviewCues(0);
    const next = copyScene(catalog.scenes[name]);
    setScene(next);
    setPreviewKind("scene");
    setSelectedLibraryScene(name);
    setSelectedEventId(next.timeline[0]?.id ?? null);
    setCurrentTime(0);
    setPlaying(false);
    setDirty(false);
    setSaveAsName(`${name}_studio`);
    setNotice(next.source === "built_in"
      ? `Loaded built-in scene “${name}” as a read-only source. Save As creates a user copy.`
      : next.remote_revision
        ? `Loaded Pi user scene “${name}”. Save changes uses revision checking; Save As creates a copy.`
        : `Loaded local user scene “${name}”. Save As creates another local or Pi copy.`);
  };

  const previewPose = (name: string) => {
    resetPreviewCues(0);
    const next: SceneDefinition = {
      format_version: 1,
      name: `${name}_pose_preview`,
      description: `Studio preview of the ${name} pose.`,
      source: "draft",
      timeline: [{ id: crypto.randomUUID(), at: 0, type: "goto_pose", pose: name, duration_seconds: 1 }],
    };
    setScene(next);
    setPreviewKind("pose");
    setSelectedLibraryScene("");
    setSelectedEventId(next.timeline[0].id);
    setCurrentTime(1);
    setDirty(false);
    setSaveAsName(`${name}_scene`);
    setNotice(`Viewing named pose “${name}”.`);
  };

  const previewMotion = (name: string) => {
    resetPreviewCues(0);
    const next: SceneDefinition = {
      format_version: 1,
      name: `${name}_motion_preview`,
      description: `Studio preview of the ${name} motion.`,
      source: "draft",
      timeline: [{ id: crypto.randomUUID(), at: 0, type: "play_motion", motion: name }],
    };
    setScene(next);
    setPreviewKind("motion");
    setSelectedLibraryScene("");
    setSelectedEventId(next.timeline[0].id);
    setCurrentTime(0);
    setPlaying(false);
    setDirty(false);
    setSaveAsName(`${name}_scene`);
    setNotice(`Viewing authored motion “${name}” with Studio’s quintic preview sampler.`);
  };

  const newScene = () => {
    resetPreviewCues(0);
    const next: SceneDefinition = {
      format_version: 1,
      name: "untitled_scene",
      description: "A new Orion Studio scene.",
      source: "draft",
      timeline: [createEvent("light", 0, catalog)],
    };
    setScene(next);
    setPreviewKind("scene");
    setSelectedLibraryScene("");
    setSelectedEventId(next.timeline[0].id);
    setCurrentTime(0);
    setDirty(true);
    setSaveAsName("my_orion_scene");
    setNotice("Created a new scene draft. Add semantic clips, then Save As to scenes/user/.");
  };

  const updateEvent = (updated: SceneEvent) => {
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline(current.timeline.map((event) => event.id === updated.id ? updated : event)),
    }));
    setDirty(true);
  };

  const moveEvent = (id: string, at: number) => {
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline(current.timeline.map((event) => event.id === id ? { ...event, at } : event)),
    }));
    setCurrentTime(at);
    setPlaying(false);
    setDirty(true);
  };

  const updateSceneDescription = (description: string) => {
    setScene((current) => ({ ...current, description, source: "draft" }));
    setDirty(true);
  };

  const deleteEvent = () => {
    if (!selectedEventId || scene.timeline.length === 1) {
      setNotice("A scene must keep at least one event.");
      return;
    }
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: current.timeline.filter((event) => event.id !== selectedEventId),
    }));
    setSelectedEventId(null);
    setDirty(true);
  };

  const addEvent = (type: SceneEvent["type"]) => {
    const event = createEvent(type, Number(currentTime.toFixed(2)));
    setScene((current) => ({
      ...current,
      source: "draft",
      timeline: sortedTimeline([...current.timeline, event]),
    }));
    setSelectedEventId(event.id);
    setDirty(true);
  };

  const saveUserScene = async () => {
    const name = saveAsName.trim();
    if (!/^[A-Za-z0-9_-]+$/.test(name)) {
      setNotice("Save As name may contain only letters, numbers, underscores, and hyphens.");
      return;
    }
    const document = sceneDocument(scene, name);
    try {
      const result = connected
        ? await publishScene(connection, document)
        : await invoke<{ name: string; relative_path: string }>("save_user_scene", {
          projectRoot,
          document,
        });
      const savedScene: SceneDefinition = {
        ...copyScene(scene),
        name: result.name,
        source: "user",
        remote_revision: "revision" in result && typeof result.revision === "string"
          ? result.revision
          : undefined,
      };
      setScene(savedScene);
      setSceneCatalog((current) => ({ ...current, [savedScene.name]: savedScene }));
      setSelectedLibraryScene(savedScene.name);
      setDirty(false);
      setSaveAsName(`${savedScene.name}_copy`);
      setNotice(
        `Saved ${result.relative_path}${connected ? " on Orion" : " in the local project"}. Studio did not modify any built-in scene.`,
      );
    } catch (error) {
      setNotice(`Could not save scene: ${String(error)}`);
    }
  };

  const saveSceneChanges = async () => {
    if (!connected || !scene.remote_revision) {
      setNotice("Connect to Orion and load a Pi user scene before saving changes in place.");
      return;
    }
    try {
      const result = await updateUserScene(
        connection,
        scene.name,
        scene.remote_revision,
        sceneDocument(scene, scene.name),
      );
      const savedScene: SceneDefinition = {
        ...copyScene(scene),
        source: "user",
        remote_revision: result.revision,
      };
      setScene(savedScene);
      setSceneCatalog((current) => ({ ...current, [savedScene.name]: savedScene }));
      setDirty(false);
      setNotice(`Saved revision-checked changes to ${result.relative_path} and reloaded Orion's scene catalog.`);
    } catch (error) {
      setNotice(`Could not save changes: ${String(error)}`);
    }
  };

  const connectRobot = async () => {
    try {
      const [status, capabilities, remoteLibrary] = await Promise.all([
        getStatus(connection),
        getCapabilities(connection),
        listUserScenes(connection),
      ]);
      if (status.api_version !== 1 || capabilities.api_version !== 1 || remoteLibrary.api_version !== 1) {
        throw new Error("Studio requires Orion gateway protocol version 1.");
      }
      const remoteSources = await Promise.all(
        remoteLibrary.scenes.map((item) => getUserScene(connection, item.name)),
      );
      const remoteScenes = remoteSources.map(remoteScene);
      setSceneCatalog((current) => {
        const next = { ...current };
        for (const remote of remoteScenes) {
          if (next[remote.name]?.source !== "built_in") next[remote.name] = remote;
        }
        return next;
      });
      setRobotStatus(status);
      setRobotCapabilities(capabilities);
      setConnected(true);
      setConnectionOpen(false);
      localStorage.setItem("orionStudioGateway", gatewayUrl);
      sessionStorage.setItem("orionStudioToken", gatewayToken);
      setNotice(
        `Connected to Orion in ${status.runtime.mode} mode · loaded ${remoteScenes.length} Pi user scenes · ${capabilities.capabilities.scene.length} runnable scenes · ${capabilities.capabilities.motion.length} motions · ${capabilities.capabilities.goto.length} poses.`,
      );
    } catch (error) {
      setConnected(false);
      setRobotCapabilities(null);
      setNotice(`Could not connect to Orion: ${String(error)}`);
    }
  };

  const runOnRobot = async () => {
    if (!connected) {
      setConnectionOpen(true);
      setNotice("Connect to Orion before running hardware capabilities.");
      return;
    }
    try {
      if (previewKind === "pose") {
        const event = scene.timeline.find((item) => item.type === "goto_pose");
        if (event?.type === "goto_pose") await gotoPose(connection, event.pose, event.duration_seconds);
      } else if (previewKind === "motion") {
        const event = scene.timeline.find((item) => item.type === "play_motion");
        if (event?.type === "play_motion") await runMotion(connection, event.motion);
      } else {
        if (dirty || scene.source === "draft") {
          throw new Error("Save As before hardware playback so Studio and Orion use the same scene document.");
        }
        if (scene.source === "user" && !scene.remote_revision) {
          const published = await publishScene(connection, sceneDocument(scene, scene.name));
          const synchronized = { ...scene, remote_revision: published.revision };
          setScene(synchronized);
          setSceneCatalog((current) => ({ ...current, [scene.name]: synchronized }));
        }
        await runScene(connection, scene.name);
      }
      setNotice(`Orion accepted the ${previewKind} capability. Following its measured result…`);
      setRobotStatus(await getStatus(connection));
    } catch (error) {
      setNotice(`Hardware run was not started: ${String(error)}`);
    }
  };

  const stopRobot = async () => {
    if (!robotStatus || !connected) return;
    try {
      if (robotStatus.scene.active) {
        await cancelRun(connection, "scene", robotStatus.scene.active.run_id);
      } else if (robotStatus.runtime.motion) {
        await cancelRun(connection, "movement", robotStatus.runtime.motion.run_id);
      }
      setRobotStatus(await getStatus(connection));
      setNotice("Cancellation was acknowledged by Orion.");
    } catch (error) {
      setNotice(`Cancellation failed: ${String(error)}`);
    }
  };

  return (
    <div className="studio-shell">
      <header className="topbar">
        <div className="brand-lockup">
          <div className="brand-mark"><Sparkles size={18} /></div>
          <div><strong>ORION</strong><span>STUDIO</span></div>
        </div>
        <div className="project-chip"><CloudCog size={15} /><span>{projectRoot || "Resolving Orion project…"}</span></div>
        <div className="topbar-actions">
          <button className="secondary-button" onClick={() => setConnectionOpen((open) => !open)}>
            {connected ? <Radio size={16} /> : <Unplug size={16} />}
            {connected ? robotStatus?.runtime.mode ?? "Connected" : "Connect robot"}
          </button>
          {scene.remote_revision && dirty && <button className="secondary-button" onClick={saveSceneChanges}><Save size={16} />Save changes</button>}
          <button className="secondary-button" onClick={saveUserScene}><Save size={16} />Save As</button>
          <button className="primary-button" onClick={runOnRobot}><Link2 size={16} />{scene.source === "user" && !scene.remote_revision && !dirty ? "Publish & Run" : "Run on Orion"}</button>
          <button className="stop-button" onClick={stopRobot} disabled={!robotStatus?.scene.active && !robotStatus?.runtime.motion}><CircleStop size={16} />Stop</button>
        </div>
      </header>

      {connectionOpen && (
        <section className="connection-popover">
          <header><div><p className="panel-kicker">ROBOT LINK</p><h2>Connect to Orion</h2></div><span>Pi gateway v1</span></header>
          <label>Gateway URL<input value={gatewayUrl} onChange={(event) => setGatewayUrl(event.target.value)} /></label>
          <label>Studio token<input type="password" value={gatewayToken} onChange={(event) => setGatewayToken(event.target.value)} /></label>
          <button className="primary-button" onClick={connectRobot}>Connect</button>
          <small>The raw `/tmp/oriond.sock` remains private on the Pi.</small>
        </section>
      )}

      <main className="studio-workspace">
        <nav className="asset-browser" aria-label="Orion asset library">
          <div className="browser-heading"><div><p className="panel-kicker">LIBRARY</p><h2>Orion assets</h2></div><button className="icon-button" onClick={newScene} aria-label="New scene"><Plus size={17} /></button></div>

          <section className="asset-section">
            <h3>Scenes <span>{Object.keys(catalog.scenes).length}</span></h3>
            <div className="asset-list">
              {Object.values(catalog.scenes).map((item) => (
                <button key={item.name} className={`asset-item ${selectedLibraryScene === item.name ? "active" : ""}`} onClick={() => loadScene(item.name)}>
                  <Sparkles size={14} /><span>{item.name.replaceAll("_", " ")}</span>
                </button>
              ))}
            </div>
          </section>

          <section className="asset-section">
            <h3>Poses <span>{Object.keys(catalog.poses).length}</span></h3>
            <div className="asset-list compact">
              {Object.keys(catalog.poses).map((name) => (
                <button key={name} className="asset-item" onClick={() => previewPose(name)}><Move3d size={14} /><span>{name.replaceAll("_", " ")}</span></button>
              ))}
            </div>
          </section>

          <section className="asset-section">
            <h3>Motions <span>{Object.keys(catalog.motions).length}</span></h3>
            <div className="asset-list compact">
              {Object.keys(catalog.motions).map((name) => (
                <button key={name} className="asset-item" onClick={() => previewMotion(name)}><Move3d size={14} /><span>{name.replaceAll("_", " ")}</span></button>
              ))}
            </div>
          </section>
        </nav>

        <section className="canvas-column">
          <div className="scene-heading">
            <div>
              <p className="panel-kicker">{previewKind.toUpperCase()} PREVIEW</p>
              <h1>{scene.name.replaceAll("_", " ")}</h1>
              <p>{scene.description}</p>
            </div>
            <div className="scene-flags">
              <span className={`source-badge ${scene.source}`}>{scene.source.replaceAll("_", " ")}</span>
              {dirty && <span className="dirty-badge">Unsaved edits</span>}
            </div>
          </div>
          <RobotViewport catalog={catalog} joints={previewJoints} light={previewLight} />
          <div className="add-event-bar">
            <span>Add at {currentTime.toFixed(2)}s</span>
            <button onClick={() => addEvent("play_motion")}><Move3d size={14} />Motion</button>
            <button onClick={() => addEvent("goto_pose")}><Move3d size={14} />Pose</button>
            <button onClick={() => addEvent("light")}><Lightbulb size={14} />Light</button>
            <button onClick={() => addEvent("audio")}><Music2 size={14} />Cue</button>
          </div>
        </section>

        <div className="inspector-column">
          <section className="scene-save-card">
            <p className="panel-kicker">SAFE SAVE</p>
            <label>Save As name<input value={saveAsName} onChange={(event) => setSaveAsName(event.target.value)} /></label>
            <label>Scene description<textarea rows={3} value={scene.description} onChange={(event) => updateSceneDescription(event.target.value)} /></label>
            <p>{connected ? "Pi-authoritative" : "Offline local staging"} · <code>scenes/user/</code></p>
          </section>
          <EventInspector catalog={catalog} event={selectedEvent} onChange={updateEvent} onDelete={deleteEvent} />
          <section className="robot-status-card">
            <div><Radio size={15} /><strong>{connected ? "Orion connected" : "Robot offline"}</strong></div>
            <p>{connected
              ? `Runtime ${robotStatus?.runtime.mode} · build ${robotStatus?.runtime.build_revision ?? "unknown"} · ${robotRunSummary}`
              : "Open Connect robot to pair with the Pi gateway."}</p>
            {robotCapabilities && <p>{robotCapabilities.capabilities.scene.length} scenes · {robotCapabilities.capabilities.motion.length} motions · {robotCapabilities.capabilities.goto.length} poses on Pi</p>}
          </section>
        </div>
      </main>

      <Timeline
        catalog={catalog}
        scene={scene}
        duration={duration}
        currentTime={currentTime}
        playing={playing}
        selectedEventId={selectedEventId}
        onSelectEvent={setSelectedEventId}
        onMoveEvent={moveEvent}
        onSeek={(time) => { resetPreviewCues(time); setCurrentTime(time); setPlaying(false); }}
        onTogglePlay={() => {
          if (currentTime >= duration) {
            resetPreviewCues(0);
            setCurrentTime(0);
          }
          setPlaying((value) => !value);
        }}
        onRewind={() => { resetPreviewCues(0); setCurrentTime(0); setPlaying(false); }}
      />

      <footer className="statusbar">
        <span className="status-pulse" />
        <p>{notice}</p>
        <div><Volume2 size={13} />{previewCue ? `Previewing ${previewCue}` : `${catalog.cues.length} cue · ${scene.timeline.length} events`}</div>
      </footer>
    </div>
  );
}
