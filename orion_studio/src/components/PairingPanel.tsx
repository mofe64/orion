import { useEffect, useRef, useState } from "react";
import { Link2, Unplug, X } from "lucide-react";
import type { ConnectionSnapshot, PairingController } from "../lib/pairing";

export function PairingPanel({ controller, state, onClose }: {
  controller: PairingController; state: ConnectionSnapshot; onClose: () => void;
}) {
  const [address, setAddress] = useState(state.address ?? localStorage.getItem("orionStudioGateway") ?? "http://orion.local:7447");
  const [token, setToken] = useState("");
  const [editing, setEditing] = useState(!state.paired || state.phase === "auth_required");
  useEffect(() => { if (state.phase === "auth_required") setEditing(true); }, [state.phase]);
  const panel = useRef<HTMLElement>(null);
  const busy = state.phase === "loading" || state.phase === "connecting";
  const needsPairing = editing || !state.paired || state.phase === "auth_required";
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    panel.current?.querySelector<HTMLElement>("button, input")?.focus();
    return () => previous?.focus();
  }, []);
  return <section ref={panel} id="orion-pairing" className="connection-popover" role="dialog"
    aria-labelledby="pairing-title" onKeyDown={(event) => { if (event.key === "Escape") onClose(); }}>
    <header><h2 id="pairing-title">{needsPairing ? state.persistent ? "Pair with Orion" : "Connect to Orion" : "Your Orion"}</h2>
      <button className="icon-button" aria-label="Close connection settings" onClick={onClose}><X size={16} /></button></header>
    <p className="field-help">{needsPairing
      ? state.persistent ? "Pair once. Studio saves the connection securely on this computer and reconnects when Orion is available."
        : "This browser connection lasts for this tab only. Use the desktop app to pair once and remember Orion."
      : state.address}</p>
    <p role="status" aria-live="polite" className={state.error ? "voice-error" : "field-help"}>
      {state.error ?? (state.phase === "connected" ? state.persistent ? "Connected · pairing saved" : "Connected · this session only" : state.phase === "disconnected"
        ? state.persistent ? "Disconnected for this session. Your pairing is still saved." : "Disconnected for this session." : busy ? "Connecting…" : "Use the token provided during Orion setup.")}</p>
    {needsPairing ? <form onSubmit={(event) => {
      event.preventDefault();
      void controller.pair(address, token).then((saved) => { if (saved) { setToken(""); onClose(); } });
    }}>
      <label>Orion address<input value={address} required autoComplete="url" spellCheck={false} onChange={(event) => setAddress(event.target.value)} /></label>
      <label>Pairing token<input type="password" value={token} required minLength={32} maxLength={4096} autoComplete="off" spellCheck={false} onChange={(event) => setToken(event.target.value)} /></label>
      <button className="primary-button" type="submit" disabled={busy}><Link2 size={15} />{busy ? "Connecting…" : state.persistent ? "Pair and remember Orion" : "Connect for this session"}</button>
    </form> : <>
      {state.phase === "connected" || state.phase === "reconnecting" || busy
        ? <button className="secondary-button" onClick={() => controller.disconnect()}><Unplug size={15} />Disconnect</button>
        : <button className="primary-button" onClick={() => { void controller.reconnect(); }}><Link2 size={15} />Reconnect</button>}
    </>}
    {!state.paired && state.phase === "error" && <button className="secondary-button" onClick={() => { void controller.start(); }}>Retry saved pairing</button>}
    {(state.paired || state.phase === "error") && <button className="quiet-button" disabled={busy} onClick={() => { setToken(""); void controller.forget(); }}>{state.persistent ? "Forget Orion on this computer" : "Clear this connection"}</button>}
    <small className="field-help">Connecting does not turn on the microphone or start a movement.</small>
  </section>;
}
