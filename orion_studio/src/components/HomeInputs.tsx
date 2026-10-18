import { useId, useState, type ComponentProps } from "react";
import { Eye, EyeOff, ExternalLink } from "lucide-react";
import { isTauri } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

export function PasswordInput({ label, ...props }: ComponentProps<"input"> & { label: string }) {
  const id = useId();
  const [visible, setVisible] = useState(false);
  return <div className="home-password"><label htmlFor={id}>{label}</label><div><input {...props} id={id} type={visible ? "text" : "password"}/><button type="button" className="quiet-button" aria-label={`${visible ? "Hide" : "Show"} ${label.toLowerCase()}`} aria-pressed={visible} aria-controls={id} onClick={() => setVisible(value => !value)}>{visible ? <EyeOff size={17} aria-hidden="true"/> : <Eye size={17} aria-hidden="true"/>}<span>{visible ? "Hide" : "Show"}</span></button></div></div>;
}
export function HomeAssistantLink({ url, children }: { url: string; children: string }) {
  const [error, setError] = useState(false);
  return <div className="home-external"><a href={url} target="_blank" rel="noreferrer" onClick={event => {
    if (!isTauri()) return;
    event.preventDefault(); setError(false);
    const parsed = new URL(url);
    if (!["http:", "https:"].includes(parsed.protocol) || parsed.username || parsed.password) { setError(true); return; }
    void openUrl(parsed.href).catch(() => setError(true));
  }}>{children}<ExternalLink size={14} aria-hidden="true"/></a>{error && <p className="settings-error" role="alert">Could not open your browser. Try the link again.</p>}</div>;
}
