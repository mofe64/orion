import { describe, expect, it, vi } from "vitest";
import { createMusicControl, validSpotifyClientId } from "./music";
import type { MusicStatus } from "./gateway";

const connection = {url:"http://lamp",token:"test"};
const status = (signed_in = false): MusicStatus => ({signed_in,client_id:"a".repeat(32),name:"Orion",redirect_uri:"http://127.0.0.1:49687/callback",now_playing:null,error:null});
const tick = async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); };

describe("Spotify Studio flow", () => {
  it("validates the client ID without accepting a secret or partial ID", () => {
    expect(validSpotifyClientId("a".repeat(32))).toBe(true);
    for (const value of ["", "a".repeat(31),"g".repeat(32),"client-secret"]) expect(validSpotifyClientId(value)).toBe(false);
  });
  it("waits for native sign-in, then loads the lamp's account state", async () => {
    const get = vi.fn().mockResolvedValue(status());
    let complete!: () => void;
    const signIn = vi.fn(() => new Promise<void>(resolve => {complete=resolve;}));
    const control = createMusicControl(connection,{get,signIn,command:vi.fn()});
    control.start(); await tick();
    const signingIn = control.signIn("a".repeat(32));
    expect(control.snapshot().pending).toBe(true); expect(control.snapshot().status?.signed_in).toBe(false);
    get.mockResolvedValue(status(true)); complete(); await signingIn;
    expect(control.snapshot().status?.signed_in).toBe(true); expect(control.snapshot().pending).toBe(false); control.stop();
  });
  it("shows Spotify's plain instructions and preserves the last known song on command failure", async () => {
    const initial = {...status(true),now_playing:{title:"Song",artist:"Artist",cover:null,playing:true}};
    const control = createMusicControl(connection,{get:vi.fn().mockResolvedValue(initial),signIn:vi.fn(),command:vi.fn().mockRejectedValue(new Error("Spotify Premium is required to play music on Orion."))});
    control.start(); await tick(); await control.command("pause");
    expect(control.snapshot().error).toContain("Premium"); expect(control.snapshot().status?.now_playing?.title).toBe("Song"); control.stop();
  });
  it("ignores a sign-in result after the connection changes", async () => {
    let complete!: () => void;
    const control = createMusicControl(connection,{get:vi.fn().mockResolvedValue(status()),signIn:() => new Promise<void>(resolve => {complete=resolve;}),command:vi.fn()});
    control.start(); await tick(); const signingIn=control.signIn("a".repeat(32)); control.stop(); complete(); await signingIn;
    expect(control.snapshot().status?.signed_in).toBe(false);
  });
});
