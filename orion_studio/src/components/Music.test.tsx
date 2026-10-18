import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { Music, SpotifySetup } from "./Music";

it("guides setup with an exact loopback address, dashboard, copy and Client ID", () => {
  const html=renderToStaticMarkup(<SpotifySetup pending={false} onSignIn={vi.fn()}/>);
  expect(html).toContain("https://developer.spotify.com/dashboard");
  expect(html).toContain("http://127.0.0.1:49687/callback"); expect(html).not.toContain("localhost");
  expect(html).toContain('aria-label="Copy Spotify redirect URI"'); expect(html).toContain("Client ID");
  expect(html).toMatch(/disabled=""[^>]*>Sign in with Spotify/);
  expect(html).not.toMatch(/PKCE|refresh token|PCM|client secret/i);
});
it("explains Spotify app playback and includes the shared master volume", () => {
  const html=renderToStaticMarkup(<Music connection={{url:"http://lamp",token:"test"}} audio={{volume:40,muted:false,epoch:"one",revision:0,save_error:null}}/>);
  expect(html).toContain("Play to Orion from the Spotify app without signing in here.");
  expect(html).toContain('aria-label="Volume controls"'); expect(html).toContain("40%");
});
