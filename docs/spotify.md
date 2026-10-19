# Spotify

Orion appears as a Spotify Connect speaker on the same Wi-Fi network. Its default speaker name is Orion.
Spotify Premium is required. Playing from the Spotify app does not require setup in Studio.

## Play from Spotify

1. Turn Orion on.
2. Connect your phone or computer to the same Wi-Fi network as Orion.
3. Open Spotify.
4. Start a song.
5. Open the speaker list.
6. Select Orion.

Orion mixes music with speech, cues and alarms through its USB speaker output.
Music becomes quieter during a voice request, speech, cues and alarms. Its previous level returns smoothly afterward.
The Volume slider controls all sound. Alarms remain audible when Orion is muted.

## Set up music by voice

Studio uses a Client ID from your Spotify app. A Client ID identifies your app to Spotify.
Spotify’s sign-in page returns to Studio through a redirect URI. Studio shows the exact address to copy.

1. Connect Orion in the Studio desktop app.
2. Open Music on the Orion page.
3. Select **Set up music by voice**.
4. Select **Open Spotify dashboard**.
5. Sign in with your Spotify Premium account.
6. Create an app.
7. Enter an app name and description.
8. Select **Web API**.
9. Copy the redirect URI from Studio.
10. Paste it into the app’s redirect URI field.
11. Save the app settings.
12. Copy the app’s Client ID.
13. Paste the Client ID into Studio.
14. Select **Sign in with Spotify**.
15. Complete sign-in in your browser.
16. Return to Studio.

The redirect URI uses `127.0.0.1`. Spotify does not accept `localhost`.
The Client ID contains 32 hexadecimal characters. Studio does not need a client secret.
The lamp stores the sign-in credentials. Studio receives no refresh token.

## Use music by voice

Say “Hey Orion, play” followed by a song, artist, album or playlist.
Say “Pause”, “Resume”, “Skip”, “Previous song” or “What’s playing?” to control music.
Say “Volume 40”, “Louder” or “Quieter” to change volume.

## Dance to music

Orion sways and turns with music phrases. Strong beats can add a head accent and small arm movements.
Calm music produces smaller movements. Voice requests, speech, tracking, work, rest and alarms take priority.
Orion returns to its held pose during silence. Its usual idle movements continue when it does not dance.

Use **Dance to music** in Music or Behaviour settings.

| Setting | Function |
| --- | --- |
| Off | Keep usual movements while music plays. |
| Sometimes | Dance for 20–40 seconds, then take a 1–3 minute break. |
| Always | Dance while music has a clear beat. |

Sometimes is the default. Higher energy music produces more frequent dance stretches.
Say “Dance sometimes”, “Always dance to music” or “Stop dancing” to change the setting.
Orion stores the setting on the lamp.

The speaker reconnects after a USB interruption. Spotify remains available while the speaker returns.

## Correct a sign-in problem

For a wrong redirect URI, copy the complete address from Studio into the Spotify app settings.
Save the settings before you sign in again.

If Spotify refuses your account, open **User Management** in your app dashboard.
Add the Spotify account that you use for sign-in.

If Spotify reports a Premium requirement, use a Spotify Premium account.
If Orion is missing, open Spotify on the same Wi-Fi and select Orion from the speaker list.

## Release and playback

The release build compiles librespot from the source version and checksum in `scripts/release-spotify.json`.
The release includes the binary, its checksum and its licence. The lamp does not compile it.
`orion-spotify.service` starts discovery at boot. It sends raw stereo audio to `oriond` through a private local socket.
`oriond` owns the only speaker client. It applies volume and ducking before USB playback.
The XVF3800 receives the mixed audio for echo cancellation. Its PCM controls remain at unity gain.

The mixer estimates beats and energy from music samples before playback. Three filter bands detect sound increases without a network request.
The character controller generates small relative movements through the normal motion compiler. Existing joint limits and activity priorities apply.
