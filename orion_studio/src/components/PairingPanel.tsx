import { useEffect, useRef, useState } from "react";
import { Link2, Plus, Unplug, X } from "lucide-react";
import type { ConnectionSnapshot, ConnectionStep, PairingController } from "../lib/pairing";

const connectionMessages: Record<ConnectionStep, string> = {
  credentials: "Waiting for secure credential storage. Check for a system permission dialog.",
  code: "Checking the pairing code with Orion…",
  verification: "Checking Orion’s status and capabilities…",
  saving: "Saving the connection securely. Check for a system permission dialog.",
};
const connectionLabels: Record<ConnectionStep, string> = {
  credentials: "Checking saved connections…", code: "Checking code…",
  verification: "Checking Orion…", saving: "Saving connection…",
};

export function PairingPanel({ controller, state, onClose }: {
  controller: PairingController; state: ConnectionSnapshot; onClose: () => void;
}) {
  const [address, setAddress] = useState(state.address ?? localStorage.getItem("orionStudioGateway") ?? "http://orion.local:7447");
  const [token, setToken] = useState("");
  const [useToken, setUseToken] = useState(false);
  const [code, setCode] = useState("");
  const [codeRequested, setCodeRequested] = useState(false);
  const [requestingCode, setRequestingCode] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const attempt = useRef(0);
  const [codeMessage, setCodeMessage] = useState<string | null>(null);
  const [editing, setEditing] = useState(!state.paired || state.phase === "auth_required");
  const [changingAddress, setChangingAddress] = useState(false);
  const [addingLamp, setAddingLamp] = useState(false);
  const otherLamps = (state.lamps ?? []).filter((lamp) => !lamp.active);
  useEffect(() => { if (state.phase === "auth_required") setEditing(true); }, [state.phase]);
  const panel = useRef<HTMLElement>(null);
  // Reconnecting a saved lamp must not lock the form for a different address.
  const busy = requestingCode || submitting;
  const backgroundConnecting = state.phase === "loading" || state.phase === "connecting";
  const needsPairing = editing || addingLamp || !state.paired || state.phase === "auth_required";
  const resetCode = () => { setCode(""); setCodeRequested(false); setCodeMessage(null); };
  const requestCode = async () => {
    const current = ++attempt.current;
    setRequestingCode(true);
    setCodeMessage(null);
    try {
      const result = await controller.requestCode(address);
      if (current === attempt.current && result) {
        setCode(""); setCodeRequested(true);
        setCodeMessage(result.message ?? null);
      }
    } finally {
      if (current === attempt.current) setRequestingCode(false);
    }
  };
  const submit = async () => {
    const current = ++attempt.current;
    setSubmitting(true);
    try {
      const saved = await (changingAddress ? controller.changeAddress(address) : useToken
        ? controller.pair(address, token) : controller.pairWithCode(address, code));
      if (current === attempt.current && saved) {
        setToken(""); resetCode(); setAddingLamp(false); onClose();
      }
    } finally {
      if (current === attempt.current) setSubmitting(false);
    }
  };
  const cancel = () => {
    ++attempt.current;
    controller.cancelCodeRequest();
    if (submitting || backgroundConnecting) controller.disconnect();
    setRequestingCode(false); setSubmitting(false); resetCode();
  };
  useEffect(() => () => {
    ++attempt.current;
    controller.cancelCodeRequest();
  }, [controller]);
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
      {requestingCode ? "Listen to Orion…" : state.error ?? (state.step ? connectionMessages[state.step] : null) ?? codeMessage ?? (needsPairing && !changingAddress
        ? useToken ? "Use the token provided during Orion setup." : "Orion will say a 6-digit code. Type it here."
        : state.phase === "connected" ? state.persistent ? "Connected · connection saved" : "Connected · this session only" : state.phase === "disconnected"
        ? state.persistent ? "Disconnected for this session. Your connection is still saved." : "Disconnected for this session." : busy || backgroundConnecting ? "Connecting…" : "Use the token provided during Orion setup.")}</p>
    {needsPairing || changingAddress ? <form onSubmit={(event) => {
      event.preventDefault();
      if (busy) return;
      if (!changingAddress && !useToken && !codeRequested) { void requestCode(); return; }
      void submit();
    }}>
      <label>Orion address<input value={address} required autoComplete="url" spellCheck={false} disabled={busy} onChange={(event) => { setAddress(event.target.value); resetCode(); }} /></label>
      {!changingAddress && (useToken
        ? <label>Connection token<input type="password" value={token} required minLength={32} maxLength={4096} autoComplete="off" spellCheck={false} onChange={(event) => setToken(event.target.value)} /></label>
        : codeRequested && <label>Pairing code<input value={code} required minLength={6} maxLength={6} pattern="[0-9]{6}" inputMode="numeric" autoComplete="one-time-code" spellCheck={false} onChange={(event) => setCode(event.target.value.replace(/[^0-9]/g, "").slice(0, 6))} /></label>)}
      <button className="primary-button" type="submit" disabled={busy}><Link2 size={15} />{requestingCode ? "Listen to Orion…" : busy ? state.step ? connectionLabels[state.step] : "Connecting…" : changingAddress ? "Save address and connect" : useToken
        ? state.persistent ? "Connect and remember Orion" : "Connect for this session" : codeRequested ? "Pair" : "Get code"}</button>
      {!changingAddress && !useToken && codeRequested && <button className="quiet-button" type="button" disabled={busy} onClick={() => { void requestCode(); }}>Say it again</button>}
      {!changingAddress && <button className="quiet-button" type="button" disabled={busy} onClick={() => { setUseToken(!useToken); setToken(""); resetCode(); }}>{useToken ? "Use a spoken code instead" : "Use a token instead"}</button>}
      {(changingAddress || addingLamp) && <button className="quiet-button" type="button" disabled={busy} onClick={() => { setChangingAddress(false); setAddingLamp(false); resetCode(); }}>Cancel</button>}
    </form> : <>
      {state.phase === "connected" || state.phase === "reconnecting" || backgroundConnecting
        ? <button className="secondary-button" onClick={() => controller.disconnect()}><Unplug size={15} />Disconnect</button>
        : <button className="primary-button" onClick={() => { void controller.reconnect(); }}><Link2 size={15} />Connect</button>}
      <button className="quiet-button" disabled={busy} onClick={() => {
        setAddress(state.address ?? "http://orion.local:7447");
        setChangingAddress(true);
      }}>Change address</button>
      {state.persistent && <button className="quiet-button" disabled={busy} onClick={() => {
        setAddress(""); setToken(""); setUseToken(false); resetCode(); setAddingLamp(true);
      }}><Plus size={15} />Connect another Orion</button>}
    </>}
    {(busy || (backgroundConnecting && (needsPairing || changingAddress))) && <button className="secondary-button" onClick={cancel}>
      <Unplug size={15} />{requestingCode ? "Cancel request" : "Cancel connection"}
    </button>}
    {otherLamps.length > 0 && <div className="saved-lamps" role="group" aria-label="Other saved Orions">
      <span className="field-help">Other Orions on this computer</span>
      {otherLamps.map((lamp) => <button key={lamp.url} className="secondary-button" disabled={busy}
        onClick={() => {
          setEditing(false); setChangingAddress(false); setAddingLamp(false); resetCode();
          void controller.switchTo(lamp.url);
        }}>Switch to {new URL(lamp.url).hostname}</button>)}
    </div>}
    {!state.paired && state.phase === "error" && <button className="secondary-button" onClick={() => { void controller.start(); }}>Retry connection</button>}
    {(state.paired || state.phase === "error") && <button className="quiet-button" disabled={busy} onClick={() => { setToken(""); setUseToken(false); resetCode(); setChangingAddress(false); void controller.forget(); }}>{state.persistent ? "Forget this Orion on this computer" : "Clear this connection"}</button>}
    <small className="field-help">Connecting does not turn on the microphone or start a movement.</small>
  </section>;
}
