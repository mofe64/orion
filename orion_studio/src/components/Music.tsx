import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { Copy, Music2, Pause, Play, SkipBack, SkipForward } from "lucide-react";
import type { AudioStatus } from "../types";
import type { GatewayConnection } from "../lib/gateway";
import { createMusicControl, SPOTIFY_DASHBOARD, SPOTIFY_REDIRECT_URI, validSpotifyClientId } from "../lib/music";
import { VolumeControl } from "./VolumeControl";
import "./Music.css";
import { HomeAssistantLink } from "./HomeInputs";

export function SpotifySetup({pending, onSignIn}: {pending:boolean; onSignIn:(clientId:string)=>void}) {
  const [clientId, setClientId] = useState("");
  const [copy, setCopy] = useState("");
  const valid = validSpotifyClientId(clientId);
  return <details className="music-setup"><summary>Set up music by voice</summary>
    <ol>
      <li><span>Create an app in the Spotify developer dashboard. Choose Web API.</span><HomeAssistantLink url={SPOTIFY_DASHBOARD}>Open Spotify dashboard</HomeAssistantLink></li>
      <li><span>In your Spotify app's settings, paste this address under Redirect URIs, then save.</span><div className="music-redirect"><code>{SPOTIFY_REDIRECT_URI}</code><button type="button" aria-label="Copy Spotify address" onClick={() => void navigator.clipboard.writeText(SPOTIFY_REDIRECT_URI).then(() => setCopy("Copied"), () => setCopy("Select the address above and copy it."))}><Copy size={15}/></button></div>{copy && <small role="status">{copy}</small>}</li>
      <li><label htmlFor="spotify-client-id">Paste the app’s Client ID</label><input id="spotify-client-id" type="text" value={clientId} autoComplete="off" spellCheck={false} placeholder="Paste your Client ID" aria-describedby="spotify-client-help" aria-invalid={!!clientId && !valid} onChange={event => setClientId(event.target.value)} />
        <small id="spotify-client-help">{clientId && !valid ? "Use the 32-character Client ID from your app dashboard." : "Use the Spotify Premium account that will play on Orion."}</small>
        <button className="primary-button" disabled={!valid || pending} onClick={() => onSignIn(clientId.trim())}>{pending ? "Waiting for Spotify…" : "Sign in with Spotify"}</button>
      </li>
    </ol>
    {pending && <p role="status">Finish signing in in your browser. If Spotify says the address is wrong, check that Redirect URIs has the address above. If it refuses your account, add it under User Management in the app dashboard.</p>}
  </details>;
}

export function Music({connection, audio}: {connection: GatewayConnection; audio?:AudioStatus | null}) {
  const control = useMemo(() => createMusicControl(connection), [connection]);
  const view = useSyncExternalStore(control.subscribe,control.snapshot,control.snapshot);
  useEffect(() => { control.start(); return control.stop; },[control]);
  const song = view.status?.now_playing;
  return <section className="orion-music" aria-label="Music">
    <div className="music-main"><header className="oh-panel-heading"><h2>Music</h2><span className="music-service">Spotify</span></header>
      {view.status?.signed_in ? <>
        <div className="music-now-playing">
          {song?.cover ? <img src={song.cover} alt="Album cover" width="80" height="80" /> : <div className="music-cover"><Music2 size={26} /></div>}
          <div className="music-track"><strong>{song?.title || "Ready when you are"}</strong><p>{song?.artist || "Ask Orion to play a song, artist, album or playlist."}</p>
            <div className="music-playback" role="group" aria-label="Music playback">
              <button type="button" aria-label="Previous song" disabled={view.pending} onClick={() => void control.command("previous")}><SkipBack size={18}/></button>
              <button type="button" aria-label={song?.playing ? "Pause music" : "Play music"} disabled={view.pending} onClick={() => void control.command(song?.playing ? "pause" : "resume")}>{song?.playing ? <Pause size={20}/> : <Play size={20}/>}</button>
              <button type="button" aria-label="Next song" disabled={view.pending} onClick={() => void control.command("next")}><SkipForward size={18}/></button>
            </div>
          </div>
        </div>
        <details className="music-account"><summary>Spotify account</summary><p>Signed in for music by voice.</p><button className="quiet-button" disabled={view.pending} onClick={() => void control.command("disconnect")}>Sign out</button></details>
      </> : <><p className="music-connect-hint">Play to Orion from the Spotify app without signing in here.</p><SpotifySetup pending={view.pending} onSignIn={control.signIn}/></>}
      <div className="music-movement"><div><strong>Move to music</strong><p>Small movements that follow the song.</p></div><button type="button" className="studio-switch" role="switch" aria-label="Move to music" aria-checked={view.status?.move_to_music ?? false} disabled={view.pending || view.status?.move_to_music === undefined} onClick={() => void control.movement(!view.status?.move_to_music)}><span/></button></div>
      {(view.error || view.pollError || view.status?.error) && <p className="music-error" role="alert">{view.error || view.pollError || view.status?.error}</p>}
    </div>
    <div className="music-volume"><VolumeControl connection={connection} status={audio}/></div>
  </section>;
}
