import { useEffect, useRef, useState } from "react";
import { Link2, Plus, Unplug, X } from "lucide-react";
import type { ConnectionSnapshot, PairingController } from "../lib/pairing";

export function PairingPanel({ controller, state, onClose }: {
  controller: PairingController; state: ConnectionSnapshot; onClose: () => void;
}) {
  const [address, setAddress] = useState(state.address ?? localStorage.getItem("orionStudioGateway") ?? "http://orion.local:7447");
  const [token, setToken] = useState("");
  const [useToken, setUseToken] = useState(false);
  const [code, setCode] = useState("");
  const [codeRequested, setCodeRequested] = useState(false);
  const [requestingCode, setRequestingCode] = useState(false);
  const [codeMessage, setCodeMessage] = useState<string | null>(null);
  const [editing, setEditing] = useState(!state.paired || state.phase === "auth_required");
  const [changingAddress, setChangingAddress] = useState(false);
  const [addingLamp, setAddingLamp] = useState(false);
  const otherLamps = (state.lamps ?? []).filter((lamp) => !lamp.active);
  useEffect(() => { if (state.phase === "auth_required") setEditing(true); }, [state.phase]);
  const panel = useRef<HTMLElement>(null);
  const busy = requestingCode || state.phase === "loading" || state.phase === "connecting";
  const needsPairing = editing || addingLamp || !state.paired || state.phase === "auth_required";
  const resetCode = () => { setCode(""); setCodeRequested(false); setCodeMessage(null); };
  const requestCode = async () => {
    setRequestingCode(true);
    setCodeMessage(null);
    try {
      const result = await controller.requestCode(address);
      if (result) {
        setCode(""); setCodeRequested(true);
        setCodeMessage(result.message ?? null);
      }
    } finally {
      setRequestingCode(false);
    }
  };
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    panel.current?.querySelector<HTMLElement>("button, input")?.focus();
    return () => previous?.focus();
  }, []);
  return <section ref={panel} id="orion-pairing" className="connection-popover" role="dialog"
    aria-labelledby="pairing-title" onKeyDown={(event) => { if (event.key === "Escape") onClose(); }}>
    <header><h2 id="pairing-title">{changingAddress ? "Change Orion address" : addingLamp ? "Connect another Orion" : needsPairing ? "Connect to Orion" : "Your Orion"}</h2>
      <button className="icon-button" aria-label="Close connection settings" onClick={onClose}><X size={16} /></button></header>
    <p className="field-help">{changingAddress ? "Studio will use your existing connection token to verify this address before saving it."
      : needsPairing
      ? state.persistent ? "Connect once. Studio saves the connection securely on this computer and reconnects when Orion is available."
        : "This browser connection lasts for this tab only. Use the desktop app to connect once and remember Orion."
      : state.address}</p>
    <p role="status" aria-live="polite" className={state.error ? "voice-error" : "field-help"}>
      {requestingCode ? "Listen to Orion…" : state.error ?? codeMessage ?? (needsPairing && !changingAddress
        ? useToken ? "Use the token provided during Orion setup." : "Orion will say a 6-digit code. Type it here."
        : state.phase === "connected" ? state.persistent ? "Connected · connection saved" : "Connected · this session only" : state.phase === "disconnected"
        ? state.persistent ? "Disconnected for this session. Your connection is still saved." : "Disconnected for this session." : busy ? "Connecting…" : "Use the token provided during Orion setup.")}</p>
    {needsPairing || changingAddress ? <form onSubmit={(event) => {
      event.preventDefault();
      if (busy) return;
      if (!changingAddress && !useToken && !codeRequested) { void requestCode(); return; }
      const result = changingAddress ? controller.changeAddress(address) : useToken
        ? controller.pair(address, token) : controller.pairWithCode(address, code);
      void result.then((saved) => { if (saved) { setToken(""); resetCode(); setAddingLamp(false); onClose(); } });
    }}>
      <label>Orion address<input value={address} required autoComplete="url" spellCheck={false} disabled={requestingCode} onChange={(event) => { setAddress(event.target.value); resetCode(); }} /></label>
      {!changingAddress && (useToken
        ? <label>Connection token<input type="password" value={token} required minLength={32} maxLength={4096} autoComplete="off" spellCheck={false} onChange={(event) => setToken(event.target.value)} /></label>
        : codeRequested && <label>Pairing code<input value={code} required minLength={6} maxLength={6} pattern="[0-9]{6}" inputMode="numeric" autoComplete="one-time-code" spellCheck={false} onChange={(event) => setCode(event.target.value.replace(/[^0-9]/g, "").slice(0, 6))} /></label>)}
      <button className="primary-button" type="submit" disabled={busy}><Link2 size={15} />{requestingCode ? "Listen to Orion…" : busy ? "Connecting…" : changingAddress ? "Save address and connect" : useToken
        ? state.persistent ? "Connect and remember Orion" : "Connect for this session" : codeRequested ? "Pair" : "Get code"}</button>
      {!changingAddress && !useToken && codeRequested && <button className="quiet-button" type="button" disabled={busy} onClick={() => { void requestCode(); }}>Say it again</button>}
      {!changingAddress && <button className="quiet-button" type="button" disabled={busy} onClick={() => { setUseToken(!useToken); setToken(""); resetCode(); }}>{useToken ? "Use a spoken code instead" : "Use a token instead"}</button>}
      {(changingAddress || addingLamp) && <button className="quiet-button" type="button" disabled={busy} onClick={() => { setChangingAddress(false); setAddingLamp(false); resetCode(); }}>Cancel</button>}
    </form> : <>
      {state.phase === "connected" || state.phase === "reconnecting" || busy
        ? <button className="secondary-button" onClick={() => controller.disconnect()}><Unplug size={15} />Disconnect</button>
        : <button className="primary-button" onClick={() => { void controller.reconnect(); }}><Link2 size={15} />Connect</button>}
      <button className="quiet-button" disabled={busy} onClick={() => {
        setAddress(state.address ?? "http://orion.local:7447");
        setChangingAddress(true);
      }}>Change address</button>
      {otherLamps.length > 0 && <div className="saved-lamps" role="group" aria-label="Other saved Orions">
        <span className="field-help">Other Orions on this computer</span>
        {otherLamps.map((lamp) => <button key={lamp.url} className="secondary-button" disabled={busy}
          onClick={() => { void controller.switchTo(lamp.url); }}>Switch to {new URL(lamp.url).hostname}</button>)}
      </div>}
      {state.persistent && <button className="quiet-button" disabled={busy} onClick={() => {
        setAddress(""); setToken(""); setUseToken(false); resetCode(); setAddingLamp(true);
      }}><Plus size={15} />Connect another Orion</button>}
    </>}
    {!state.paired && state.phase === "error" && <button className="secondary-button" onClick={() => { void controller.start(); }}>Retry connection</button>}
    {(state.paired || state.phase === "error") && <button className="quiet-button" disabled={busy} onClick={() => { setToken(""); setUseToken(false); resetCode(); setChangingAddress(false); void controller.forget(); }}>{state.persistent ? "Forget this Orion on this computer" : "Clear this connection"}</button>}
    <small className="field-help">Connecting does not turn on the microphone or start a movement.</small>
  </section>;
}
