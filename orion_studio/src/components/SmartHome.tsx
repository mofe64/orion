import { useEffect, useRef, useState, type FormEvent } from "react";
import { Check, Plus, Search } from "lucide-react";
import { homeRequest, type GatewayConnection } from "../lib/gateway";
import { controllable, getHome, homeText, setupProblem, homeUrl, integrationList, optimisticEntity, type HomeCommand, type HomeDevices, type HomeFlow, type HomeStatus, type Integration } from "../lib/home";
import { visiblePolling } from "../lib/visiblePolling";
import { HomeFlowForm } from "./HomeFlow";
import { HomeAssistantLink, PasswordInput } from "./HomeInputs";
import { HomeRooms } from "./HomeDevices";
export { HomeRooms } from "./HomeDevices";
import "./Settings.css";
import "./SmartHome.css";

type Completion = { title: string; entry?: string; previous: string[] };
export function SmartHome({ connection }: { connection: GatewayConnection | null }) {
  return <ConnectedHome key={connection ? `${connection.url}:${connection.token}` : "offline"} connection={connection}/>;
}
function ConnectedHome({ connection }: { connection: GatewayConnection | null }) {
  const [status, setStatus] = useState<HomeStatus | null>(null);
  const [devices, setDevices] = useState<HomeDevices | null>(null);
  const [discovered, setDiscovered] = useState<HomeFlow[]>([]);
  const [integrations, setIntegrations] = useState<Integration[] | null>(null);
  const [query, setQuery] = useState("");
  const [flow, setFlow] = useState<HomeFlow | null>(null);
  const [flowRevision, setFlowRevision] = useState(0);
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const revision = useRef(0);
  const alive = useRef(true);
  const addButton = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  useEffect(() => { if (returnFocus.current && !busy && !flow && !integrations) { addButton.current?.focus(); returnFocus.current = false; } }, [busy, flow, integrations]);
  const beforeSetup = useRef<string[]>([]);
  const [completion, setCompletion] = useState<Completion | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [error, setError] = useState("");
  const [pollError, setPollError] = useState("");
  const [notice, setNotice] = useState("");
  const [owner, setOwner] = useState({ name: "", username: "", password: "" });
  const highlighted = useRef<Completion | null>(null);
  const roomFocus = useRef<string | null>(null);
  useEffect(() => {
    if (!roomFocus.current || busy) return;
    const select = Array.from(document.querySelectorAll<HTMLSelectElement>("[data-room-device]")).find(element => element.dataset.roomDevice === roomFocus.current);
    select?.focus(); roomFocus.current = null;
  }, [devices, busy]);
  const added = devices?.devices.filter(device => completion && (completion.entry ? device.config_entries?.includes(completion.entry) : !completion.previous.includes(device.id))).map(device => device.id) ?? [];
  useEffect(() => {
    if (!completion || !added.length || highlighted.current === completion) return;
    const heading = document.querySelector<HTMLHeadingElement>(".home-device-added h3");
    heading?.scrollIntoView({ block: "center", behavior: "instant" }); heading?.focus({ preventScroll: true });
    highlighted.current = completion;
  }, [completion, added.join(",")]);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const load = async (signal?: AbortSignal) => {
    if (!connection) return;
    const version = revision.current;
    const current = await getHome(connection, signal);
    const [rooms, found] = current.connected ? await Promise.all([homeRequest<HomeDevices>(connection, "devices", undefined, signal), homeRequest<HomeFlow[]>(connection, "discovered", undefined, signal)]) : [null, []];
    if (!alive.current || signal?.aborted || version !== revision.current) return;
    setStatus(current); setDevices(rooms); setDiscovered(found); setPollError("");
  };
  useEffect(() => {
    if (!connection) return;
    return visiblePolling(async signal => {
      if (pending.current) return;
      try { await load(signal); }
      catch { if (!signal.aborted && alive.current) setPollError("Could not reach your home. Check the lamp’s connection, then search again."); }
    }, document, 5000);
  }, [connection]);
  const act = async (task: () => Promise<void>, failure = "Could not complete that step. Check your details and try again.", device?: string, optimistic?: () => void) => {
    if (pending.current || !connection) return;
    pending.current = true; revision.current += 1;
    const previous = devices;
    setBusy(true); setError(""); setNotice("");
    if (device) setErrors(value => ({ ...value, [device]: "" }));
    optimistic?.();
    try {
      await task();
      if (alive.current) {
        try { await load(); }
        catch { setPollError("Your change was sent. Search again to check the latest device state."); }
      }
    } catch (cause) {
      if (alive.current) {
        if (!device && flow && cause instanceof Error) failure=setupProblem(cause.message,flow);
        if (optimistic) setDevices(previous);
        if (device) setErrors(value => ({ ...value, [device]: failure })); else setError(failure);
      }
    } finally { if (alive.current) setBusy(false); pending.current = false; }
  };
  const patch = (update: (device: HomeDevices["devices"][number]) => HomeDevices["devices"][number]) => setDevices(value => value && ({ ...value, devices: value.devices.map(update) }));
  const url = connection ? homeUrl(connection) : "";
  const setup = (event: FormEvent) => {
    event.preventDefault(); if (!connection) return;
    void act(async () => { try { await homeRequest(connection, "onboarding", owner); setNotice("Your home is ready. Add a device to get started."); } finally { setOwner(value => ({ ...value, password: "" })); } }, "Could not set up your account. Check your sign-in details and try again.");
  };
  const setStep = (value: HomeFlow) => {
    if (value.type === "create_entry") { setCompletion({ title: homeText(value.title, "Your device"), entry: value.result?.entry_id, previous: beforeSetup.current }); setFlow(null); }
    else { setFlow(value); setFlowRevision(revision => revision + 1); }
  };
  const begin = (task: () => Promise<HomeFlow>) => {
    beforeSetup.current = devices?.devices.map(device => device.id) ?? [];
    setCompletion(null);
    void act(async () => { setStep(await task()); setIntegrations(null); });
  };
  const search = () => void act(async () => { await load(); setNotice("Search complete. Your devices are up to date."); }, "Could not search for devices. Check the lamp’s connection and try again.");
  const openSearch = () => void act(async () => { setIntegrations(integrationList(await homeRequest(connection!, "integrations"))); setQuery(""); });
  const foundList = (ignored: boolean) => <ul className="home-found-list">{discovered.filter(item => !!item.ignored === ignored).map(found => {
    const name=homeText(found.context?.title_placeholders?.name ?? found.title ?? found.handler,"New device");
    const add=() => begin(() => found.ignored_entry_id ? homeRequest<HomeFlow>(connection!,"flows",{handler:found.handler,ignored_entry_id:found.ignored_entry_id}) : homeRequest<HomeFlow>(connection!,`flows/${encodeURIComponent(found.flow_id!)}`));
    const choose=(value:boolean) => void act(async () => {await homeRequest(connection!,"discovered",{flow_id:found.flow_id,ignored:value});});
    return <li key={found.flow_id}><div><strong>{name}</strong><p>{ignored ? found.ignored_entry_id ? "Set up this device to add it to your home." : "Undo ignore to show this device in Found nearby." : "Ready to set up"}</p></div><div className="home-actions">{ignored ? <button className="quiet-button" disabled={busy || !!flow} aria-label={found.ignored_entry_id ? `Set up ${name}` : `Undo ignore for ${name}`} onClick={() => found.ignored_entry_id ? add() : choose(false)}>{found.ignored_entry_id ? "Set up" : "Undo ignore"}</button> : <><button className="quiet-button" disabled={busy || !!flow} aria-label={`Ignore ${name}`} onClick={() => choose(true)}>Ignore</button><button className="quiet-button" disabled={busy || !!flow} aria-label={`Add ${name}`} onClick={add}><Plus size={15} aria-hidden="true"/>Add</button></>}</div></li>;
  })}</ul>;
  return <section className="settings-page smart-home" id="workspace"><header className="settings-heading home-heading"><div><p className="eyebrow">YOUR CONNECTED HOME</p><h1>Home</h1><p>Your devices, room by room.</p></div>{connection && <div className="home-heading-actions"><div className="home-actions"><button className="quiet-button" disabled={busy || !status?.connected} onClick={search}><Search size={16} aria-hidden="true"/>{busy ? "Please wait…" : "Search for devices"}</button><button ref={addButton} className="primary-button" disabled={busy || !status?.connected || !!flow} onClick={openSearch}><Plus size={17} aria-hidden="true"/>Add device</button></div><HomeAssistantLink url={url}>Open Home Assistant</HomeAssistantLink></div>}</header>
    {(error || pollError) && <div className="home-message"><p className="settings-error" role="alert">{error || pollError}</p><button className="quiet-button" disabled={busy} onClick={search}>Try again</button></div>}{notice && <p className="home-notice" role="status">{notice}</p>}
    {!connection ? <section className="settings-card home-empty"><h2>Your home starts with Orion</h2><p>Connect Orion with the connection button above, then add the devices you use every day.</p></section> : !status ? <div className="home-empty" role="status"><h2>Getting your home ready…</h2><p>Checking your devices and rooms.</p></div> : <>
      <div className="home-status"><span className={status.connected ? "home-online" : ""}>{status.connected ? "Connected to your home" : status.running ? "Ready for your account" : status.installed ? "Your home is stopped" : "Let’s set up your home"}</span>{status.installing && <span role="status">Preparing your home… This can take a few minutes.</span>}</div>
      {status.install_failed && <p role="alert" className="settings-error">Setup could not finish. Check the lamp’s internet connection, then try again below.</p>}
      {(!status.installed || !status.running) && <section className="settings-card"><h2>{status.installed ? "Start your home" : "Bring your devices together"}</h2><p>{status.installed ? "Start the home service on your lamp to reach your devices." : "Set up your lamp to find and control the devices in your home."}</p><button className="primary-button" disabled={busy || status.installing} onClick={() => void act(async () => { await homeRequest(connection, "install", {}); setStatus(previous => previous && ({ ...previous, installing: true })); })}>{status.installing ? "Preparing…" : status.installed ? "Start Home" : "Set up Home"}</button></section>}
      {status.running && !status.connected && <section className="settings-card home-account"><h2>{status.onboarded ? "Connect your home" : "Make yourself at home"}</h2><p>{status.onboarded ? "Sign in with your Home Assistant account." : "Create an owner account to keep your home private. Save these details somewhere safe."}</p><form onSubmit={setup}><fieldset disabled={busy}><label>Name<input required autoComplete="name" value={owner.name} onChange={event => setOwner(value => ({ ...value, name: event.target.value }))}/></label><label>Username<input required autoComplete="username" value={owner.username} onChange={event => setOwner(value => ({ ...value, username: event.target.value }))}/></label><PasswordInput label="Password" required autoComplete={status.onboarded ? "current-password" : "new-password"} value={owner.password} onChange={event => setOwner(value => ({ ...value, password: event.target.value }))}/><button className="primary-button">{busy ? "Connecting…" : status.onboarded ? "Connect Home" : "Create account"}</button></fieldset></form></section>}
      {status.connected && <>
        {completion && <section className="home-completion" role="status"><Check size={23} aria-hidden="true"/><div><h2>{completion.title} is ready</h2><p>Choose a room, try the controls, then turn on Voice control if you’d like Orion to help.</p>{!added.length && <p>Your device is getting ready. Search again in a moment to see its controls.</p>}</div><button className="quiet-button" onClick={() => setCompletion(null)}>Done</button></section>}
        {integrations && <section className="settings-card home-search"><header className="home-actions"><h2>Find your device</h2><button className="quiet-button" onClick={() => { setIntegrations(null); addButton.current?.focus(); }}>Close search</button></header><label>Search by brand or device name<input autoFocus type="search" value={query} onChange={event => setQuery(event.target.value)} placeholder="For example, Philips Hue"/></label><ul className="home-integrations">{integrations.filter(item => `${item.name} ${item.domain}`.toLowerCase().includes(query.toLowerCase())).map(item => <li key={item.domain}><button className="quiet-button" disabled={busy} onClick={() => begin(() => homeRequest<HomeFlow>(connection, "flows", { handler: item.domain }))}>{item.name}<Plus size={15} aria-hidden="true"/></button></li>)}</ul>{!integrations.some(item => `${item.name} ${item.domain}`.toLowerCase().includes(query.toLowerCase())) && <p>No matches. Try another name or use Home Assistant.</p>}</section>}
        {flow && <HomeFlowForm key={`${flow.flow_id}-${flow.step_id}-${flowRevision}`} flow={flow} url={url} devices={devices} busy={busy} onSubmit={data => void act(async () => setStep(await homeRequest<HomeFlow>(connection, `flows/${flow.flow_id}`, data)))} onRefresh={() => void act(async () => { try { setStep(await homeRequest<HomeFlow>(connection, `flows/${flow.flow_id}`)); } catch { const rooms = await homeRequest<HomeDevices>(connection, "devices"); setFlow(null); if (rooms.devices.some(device => !beforeSetup.current.includes(device.id))) setCompletion({ title: homeText(flow.title ?? flow.handler, "Your device"), previous: beforeSetup.current }); else setError(setupProblem("The device setup ended before it connected.",flow)); } })} onAbort={() => void act(async () => { if (flow.type !== "abort") await homeRequest(connection, `flows/${flow.flow_id}`, undefined, undefined, "DELETE"); setFlow(null); returnFocus.current = true; })}/>}
        {discovered.some(item => !item.ignored) && <section className="home-discovery"><header><div><h2>Found nearby</h2><p>A few things ready to join your home.</p></div><span>{discovered.filter(item => !item.ignored).length} found</span></header>{foundList(false)}</section>}
        <div className="home-section-heading"><h2>Your devices</h2><p>Use the controls here any time. Voice control is your choice.</p></div>
        {devices?.devices.length ? <HomeRooms devices={devices} busy={busy} errors={errors} added={added} room={(id, area) => { roomFocus.current = id; void act(async () => { await homeRequest(connection, "room", { device_id: id, area_id: area }); }, "Could not change the room. Try again.", id, () => patch(device => device.id === id ? { ...device, area_id: area, area: devices.area_registry.find(item => item.area_id === area)?.name ?? "No room yet" } : device)); }} control={(id, command: HomeCommand) => {
          const device = devices.devices.find(item => item.entities.some(entity => entity.entity_id === id));
          void act(async () => { await homeRequest(connection, "control", { entity_id: id, ...command }); }, "The device did not respond. Check it, then try again.", device?.id, () => patch(item => ({ ...item, entities: item.entities.map(entity => entity.entity_id === id ? optimisticEntity(entity, command) : entity) })));
        }} expose={(id, exposed) => void act(async () => { await homeRequest(connection, "access", { device_id: id, exposed }); }, "Could not change voice control. Try again.", id, () => patch(device => device.id === id ? { ...device, entities: device.entities.map(entity => controllable(entity) ? { ...entity, exposed } : entity) } : device))}/> : <section className="settings-card home-empty"><h2>Room for your first device</h2><p>Switch on a light, speaker or other device, then search nearby or add it by brand.</p><button className="primary-button" disabled={busy || !!flow} onClick={openSearch}>Add your first device</button></section>}
        {discovered.some(item => item.ignored) ? <details className="home-ignored"><summary>Ignored devices <span>{discovered.filter(item => item.ignored).length}</span></summary><p>Choose a device below to add it to your home or show it in Found nearby.</p>{foundList(true)}</details> : !discovered.length && <p className="settings-help home-search-hint">Looking for something else? Switch it on, then choose Search for devices.</p>}
      </>}
    </>}
  </section>;
}
