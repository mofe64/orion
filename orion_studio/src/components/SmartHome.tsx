import { useEffect, useState, type FormEvent } from "react";
import { homeRequest, type GatewayConnection } from "../lib/gateway";
import { getHome, homeUrl, integrationList, type HomeDevices, type HomeFlow, type HomeStatus, type Integration } from "../lib/home";
import { visiblePolling } from "../lib/visiblePolling";
import { HomeFlowForm } from "./HomeFlow";
import "./Settings.css";
import "./SmartHome.css";

export function HomeRooms({ devices, busy, control, expose }: { devices: HomeDevices; busy: boolean; control: (id: string, command: string) => void; expose: (id: string, enabled: boolean) => void }) {
  return <div className="home-rooms">{devices.areas.map(area => <section className="settings-card" key={area.name}><h2>{area.name}</h2><ul className="home-entities">{area.entities.map(entity => <li key={entity.entity_id}><div><strong>{entity.name}</strong><p>{entity.state.replaceAll("_", " ")} <span>· {entity.domain.replaceAll("_", " ")}</span></p></div><div className="home-entity-controls">{["light", "switch", "fan", "media_player"].includes(entity.domain) && ["on", "off"].includes(entity.state) && <button className="quiet-button" disabled={busy} onClick={() => control(entity.entity_id, entity.state === "on" ? "off" : "on")}>{entity.state === "on" ? "Turn off" : "Turn on"}<span className="sr-only"> {entity.name}</span></button>}<label className="home-check"><input type="checkbox" checked={entity.exposed} disabled={busy} onChange={event => expose(entity.entity_id, event.target.checked)}/>Orion can control<span className="sr-only"> {entity.name}</span></label></div></li>)}</ul></section>)}</div>;
}
export function SmartHome({ connection }: { connection: GatewayConnection | null }) {
  const [status, setStatus] = useState<HomeStatus | null>(null);
  const [devices, setDevices] = useState<HomeDevices | null>(null);
  const [discovered, setDiscovered] = useState<HomeFlow[]>([]);
  const [integrations, setIntegrations] = useState<Integration[] | null>(null);
  const [query, setQuery] = useState("");
  const [flow, setFlow] = useState<HomeFlow | null>(null);
  const [flowRevision, setFlowRevision] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [pollError, setPollError] = useState("");
  const [notice, setNotice] = useState("");
  const [refresh, setRefresh] = useState(0);
  const [owner, setOwner] = useState({ name: "", username: "", password: "" });
  useEffect(() => {
    setStatus(null); setDevices(null); setDiscovered([]); setIntegrations(null); setFlow(null); setOwner({ name: "", username: "", password: "" }); setError(""); setPollError(""); setNotice("");
  }, [connection]);
  useEffect(() => {
    if (!connection) return;
    return visiblePolling(async signal => {
      try {
        const current = await getHome(connection, signal);
        if (signal.aborted) return;
        setStatus(current);
        if (current.connected) {
          const [rooms, found] = await Promise.all([homeRequest<HomeDevices>(connection, "devices", undefined, signal), homeRequest<HomeFlow[]>(connection, "discovered", undefined, signal)]);
          if (!signal.aborted) { setDevices(rooms); setDiscovered(found); }
        }
        if (!signal.aborted) setPollError("");
      } catch (reason) { if (!signal.aborted) setPollError(reason instanceof Error ? reason.message : "Home is not available right now."); }
    }, document, 5000);
  }, [connection, refresh]);
  const act = async (task: () => Promise<void>) => {
    if (busy) return;
    setBusy(true); setError(""); setNotice("");
    try { await task(); setRefresh(value => value + 1); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Something went wrong. Try again."); }
    finally { setBusy(false); }
  };
  const url = connection ? homeUrl(connection) : "";
  const setup = (event: FormEvent) => {
    event.preventDefault(); if (!connection) return;
    void act(async () => { try { await homeRequest(connection, "onboarding", owner); setNotice("Home is ready."); } finally { setOwner(value => ({ ...value, password: "" })); } });
  };
  const setStep = (value: HomeFlow) => { setFlow(value); setFlowRevision(revision => revision + 1); };
  return <section className="settings-page smart-home" id="workspace"><header className="settings-heading"><p className="eyebrow">YOUR CONNECTED HOME</p><h1>Home</h1><p>Devices, rooms and access for Orion.</p>{connection && <a href={url} target="_blank" rel="noreferrer">Open Home Assistant ↗</a>}</header>
    {(error || pollError) && <p className="settings-error" role="alert">{error || pollError}</p>}{notice && <p role="status">{notice}</p>}
    {!connection ? <p>Connect Orion to set up your home.</p> : !status ? <p role="status">Loading…</p> : <>
      <div className="home-status"><span>{status.running ? "Home Assistant is running" : status.installed ? "Home Assistant is stopped" : "Home Assistant is not installed"}</span>{status.installing && <span role="status">Installing Home Assistant…</span>}</div>
      {status.install_failed && <p role="alert" className="settings-error">Home Assistant could not be installed. Check Orion’s internet connection and try again.</p>}
      {(!status.installed || !status.running) && <section className="settings-card"><h2>{status.installed ? "Start your home" : "Bring your devices together"}</h2><p>{status.installed ? "Start Home Assistant on Orion." : "Install Home Assistant on Orion to find and connect the devices in your home."}</p><button className="primary-button" disabled={busy || status.installing} onClick={() => void act(async () => { await homeRequest(connection, "install", {}); setStatus(previous => previous && ({ ...previous, installing: true })); })}>{status.installing ? "Installing…" : status.installed ? "Start Home Assistant" : "Install Home Assistant"}</button></section>}
      {status.running && !status.connected && <section className="settings-card"><h2>Set up Home</h2><p>{status.onboarded ? "Enter your Home Assistant account to connect Orion." : "Create the owner account for your home."}</p><form onSubmit={setup}><fieldset disabled={busy}><label>Name<input required autoComplete="name" value={owner.name} onChange={event => setOwner(value => ({ ...value, name: event.target.value }))}/></label><label>Username<input required autoComplete="username" value={owner.username} onChange={event => setOwner(value => ({ ...value, username: event.target.value }))}/></label><label>Password<input type="password" required autoComplete={status.onboarded ? "current-password" : "new-password"} value={owner.password} onChange={event => setOwner(value => ({ ...value, password: event.target.value }))}/></label><button className="primary-button">{busy ? "Setting up…" : "Set up Home"}</button></fieldset></form></section>}
      {status.connected && <>
        <section className="home-discovery"><header><h2>Discovered</h2><button className="quiet-button" disabled={busy} onClick={() => void act(async () => { const result = await homeRequest<unknown>(connection, "integrations"); setIntegrations(integrationList(result)); })}>Add device</button></header>{discovered.length ? <ul>{discovered.map(found => <li key={found.flow_id}><span>{found.context?.title_placeholders?.name ?? found.title ?? found.handler}</span><button className="quiet-button" disabled={busy} onClick={() => void act(async () => setStep(await homeRequest<HomeFlow>(connection, `flows/${encodeURIComponent(found.flow_id!)}`)))}>Add</button></li>)}</ul> : <p>No new devices found. Use Add device to search for one.</p>}</section>
        {integrations && <section className="settings-card"><header className="home-actions"><h2>Add device</h2><button className="quiet-button" onClick={() => setIntegrations(null)}>Close search</button></header><label>Search<input type="search" value={query} onChange={event => setQuery(event.target.value)} placeholder="Search by brand"/></label><ul className="home-integrations">{integrations.filter(item => `${item.name} ${item.domain}`.toLowerCase().includes(query.toLowerCase())).map(item => <li key={item.domain}><button className="quiet-button" disabled={busy} onClick={() => void act(async () => { setStep(await homeRequest<HomeFlow>(connection, "flows", { handler: item.domain })); setIntegrations(null); })}>{item.name}</button></li>)}</ul></section>}
        {flow && <HomeFlowForm key={`${flow.flow_id}-${flow.step_id}-${flowRevision}`} flow={flow} url={url} devices={devices} busy={busy} onSubmit={data => void act(async () => setStep(await homeRequest<HomeFlow>(connection, `flows/${flow.flow_id}`, data)))} onRefresh={() => void act(async () => setStep(await homeRequest<HomeFlow>(connection, `flows/${flow.flow_id}`)))} onAbort={() => void act(async () => { if (!["create_entry", "abort"].includes(flow.type)) await homeRequest(connection, `flows/${flow.flow_id}`, undefined, undefined, "DELETE"); setFlow(null); })}/>}
        {devices && <HomeRooms devices={devices} busy={busy} control={(id, command) => void act(async () => { await homeRequest(connection, "control", { entity_id: id, command }); setNotice("Device updated."); })} expose={(id, exposed) => void act(async () => { await homeRequest(connection, "access", { entity_id: id, exposed }); })}/>}
      </>}
    </>}
  </section>;
}
