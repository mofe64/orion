import { invoke, isTauri } from "@tauri-apps/api/core";
import { type GatewayConnection, getMusic, musicCommand, setMusicMovement } from "./gateway";

export const SPOTIFY_REDIRECT_URI = "http://127.0.0.1:49687/callback";
export const SPOTIFY_DASHBOARD = "https://developer.spotify.com/dashboard";
export const validSpotifyClientId = (value: string) => /^[a-f\d]{32}$/i.test(value.trim());
export const spotifySignIn = (clientId: string) => {
  if (!validSpotifyClientId(clientId)) return Promise.reject(new Error("Paste the 32-character Client ID from your Spotify app dashboard."));
  if (!isTauri()) return Promise.reject(new Error("Open the Orion Studio app on your computer to sign in with Spotify."));
  return invoke("spotify_sign_in", {clientId: clientId.trim()});
};

export function createMusicControl(connection: GatewayConnection,
  services = {get: getMusic, command: musicCommand, signIn: spotifySignIn, movement:setMusicMovement}) {
  let view: {status: Awaited<ReturnType<typeof getMusic>> | null; pending: boolean; error: string | null; pollError: string | null} = {status:null,pending:false,error:null,pollError:null};
  let generation = 0, revision=0, timer: ReturnType<typeof setTimeout> | undefined, abort: AbortController | undefined;
  const listeners = new Set<() => void>();
  const publish = (patch: Partial<typeof view>) => { view = {...view,...patch}; listeners.forEach(listener => listener()); };
  const refresh = async (id = generation, force = false) => {
    const observed=revision;
    try {
      const status = await services.get(connection, abort?.signal);
      if (id === generation && revision===observed && (force || !view.pending)) publish({status,pollError:null});
    } catch (error) { if (id === generation && revision===observed) publish({pollError:error instanceof Error ? error.message : String(error)}); }
  };
  const poll = async (id: number) => { await refresh(id); if (id === generation) timer = setTimeout(() => void poll(id),1000); };
  const act = async (operation: () => Promise<unknown>) => {
    if (view.pending) return;
    const id = generation; revision++; publish({pending:true,error:null});
    try { await operation(); if (id === generation) await refresh(id,true); }
    catch (error) { if (id === generation) publish({error:error instanceof Error ? error.message : String(error)}); }
    finally { if (id === generation) {revision++; publish({pending:false});} }
  };
  return {
    snapshot: () => view,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    start: () => { const id = ++generation; abort = new AbortController(); void poll(id); },
    stop: () => { generation++; clearTimeout(timer); abort?.abort(); },
    command: (action: "pause" | "resume" | "next" | "previous" | "disconnect") => act(() => services.command(connection,{action})),
    signIn: (clientId: string) => act(() => services.signIn(clientId)),
    movement: (enabled: boolean) => act(() => services.movement(connection,enabled)),
  };
}
