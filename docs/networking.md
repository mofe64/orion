# Wi-Fi fallback hotspot

Orion can expose a password-protected Wi-Fi hotspot when its normal Wi-Fi
connection fails. A laptop on that network reaches SSH at `10.42.0.1` and the
Studio gateway at `http://10.42.0.1:7447`. Codex replies and web search still
need an internet connection. Existing runtime timers, scenes and local robot
controls do not depend on an internet connection.

The fallback requires NetworkManager to own the Wi-Fi interface. Ethernet can
remain under `systemd-networkd`; NetworkManager does not need to own both.
Installing the helper does not migrate an interface between network managers.

## Install on the Pi

Run from the Orion source root on a Debian/Ubuntu Pi with NetworkManager,
`dnsmasq-base`, `iw` and system Python 3 installed. Normal Wi-Fi must already
be connected and configured to autoconnect. Verify the radio supports AP mode
and its configured country matches the robot's location:

```bash
systemctl is-active NetworkManager
nmcli -f DEVICE,TYPE,STATE,CONNECTION device status
nmcli -g WIFI-PROPERTIES.AP device show wlan0
iw reg get
sudo python3 scripts/install_pi_wifi_fallback.py --country GB --ssid Orion-Ariadne --plan
sudo python3 scripts/install_pi_wifi_fallback.py --country GB --ssid Orion-Ariadne
```

Use the actual country, interface (`--interface`) and unique SSID. The installer
prints the password only when it creates the profile. Repeat installs preserve
it. Existing profiles or files without matching installation metadata cause the
installer to stop. The installer snapshots managed files before replacing them
and restores them if activation fails. It does not activate the hotspot or
replace the current Wi-Fi connection during installation.

Installed paths:

| Path | Responsibility |
| --- | --- |
| `/usr/local/sbin/orion-wifi-fallback` | Read device state and request connection activation |
| `/etc/orion/wifi-fallback.env` | Root-owned interface, home UUID and hotspot UUID; contains no Wi-Fi passwords |
| `/etc/systemd/system/orion-wifi-fallback.service` | Root oneshot with a 50-second timeout; ordered after NetworkManager |
| `/etc/systemd/system/orion-wifi-fallback.timer` | First check at 45 seconds after boot, then 20 seconds after each run finishes, with 5-second timer accuracy |
| `/run/orion-wifi-fallback/home-attempt` | Pending home attempt across timer runs; cleared on connection success or reboot |
| `/var/lib/orion/wifi-fallback-backups/` | Installer snapshots of previously managed files |

The NetworkManager profile is named `orion-fallback-hotspot`. It uses 2.4 GHz,
WPA2/CCMP, `ipv4.method shared`, `10.42.0.1/24`, and `ipv6.method ignore`.
NetworkManager supplies DHCP through `dnsmasq-base`; no independent `dnsmasq`
or `hostapd` service is configured. The profile's autoconnect is disabled when
it is created. Only the helper activates it.

## Connection behavior

Connected devices, including an active hotspot, are left alone. Connecting and
disconnecting devices are also left alone. For a disconnected or failed device,
the helper retries the saved home UUID with a 20-second `nmcli` wait. If that
fails, it reads the device state again. It starts the hotspot only if the device
remains disconnected or failed, using another 20-second wait. A client timeout
does not prove NetworkManager has finished connecting. The helper remembers
that pending attempt across timer runs: once it fails and the device becomes
idle, the next run starts the hotspot instead of retrying home indefinitely.
Unavailable, unmanaged
or unreadable device states produce a diagnostic without changing connections.

Once active, the hotspot stays active until an explicit connection change or
reboot. It does not switch away from a connected Wi-Fi network merely because
that network has lost internet access. The 45-second timer is the first check,
not a guarantee of hotspot or application readiness; existing connection
attempts and boot synchronization can delay both.

The helper does not wait for `network-online.target`, stop or restart Orion's
application services, or change servo state. Services that wait for that target
retain their existing behavior. Gateway access requires its installed listener
to accept the hotspot's IPv4 address; the supplied `--bind ::` template does.

## Connect and return home

Join the hotspot from the laptop and keep the connection when the laptop warns
that it has no internet. Use `ssh USER@10.42.0.1`. In Studio, choose **Change
address** and enter `http://10.42.0.1:7447`; the saved token is verified and
reused. An unpaired computer can use the existing spoken-code pairing flow.
The hotspot password and Studio token are separate credentials.

To return home, make home Wi-Fi available and explicitly activate the saved
home profile. The connection change drops hotspot SSH, so keep the lamp at rest
and expect to reconnect through its normal hostname:

```bash
sudo sh -c '. /etc/orion/wifi-fallback.env; nmcli --wait 20 connection up uuid "$ORION_WIFI_HOME_UUID" ifname "$ORION_WIFI_INTERFACE"'
```

Change Studio's address back to its normal hostname. Test `.local` discovery
on each network before depending on the hostname for both connections. Check
that `10.42.0.0/24` does not conflict with the laptop's VPN or other routes.

To disable future checks:

```bash
sudo systemctl disable --now orion-wifi-fallback.timer
```

This does not deactivate a hotspot that is already running; explicitly reconnect
home to leave it. To retrieve its password again:

```bash
sudo nmcli --show-secrets -g 802-11-wireless-security.psk connection show id orion-fallback-hotspot
```

## Verify and diagnose

```bash
systemctl list-timers orion-wifi-fallback.timer --no-pager
journalctl -u orion-wifi-fallback -u NetworkManager -n 40 --no-pager
nmcli -f connection.autoconnect,802-11-wireless.mode,ipv4.method,ipv4.addresses connection show id orion-fallback-hotspot
nmcli -t -f NAME,DEVICE connection show --active
systemctl is-active oriond orion-studio-gateway orion-listener orion-voice-stack
```

Test home boot, offline boot, loss of home Wi-Fi while running, laptop DHCP,
SSH, authenticated Studio access, and explicit return home. Keep the lamp at
rest for connection switching and reboot tests. A successful connected-state
service run confirms it leaves home Wi-Fi alone; it does not prove the laptop
can associate with the hotspot.

Ariadne's live test on 9 October 2026 temporarily made the saved home access
point unavailable in NetworkManager's in-memory profile. The fallback timer
activated `Orion-Ariadne` in about 71 seconds. The laptop received
`10.42.0.18` from the Pi, connected through SSH, and received HTTP 200 from the
authenticated Studio status endpoint. An independent Pi recovery timer and a
pre-started laptop checker protected the test during the internet interruption.
Both devices returned to home Wi-Fi; saved Pi profile files were unchanged and
all four Orion application processes kept their original PIDs. The Wi-Fi radio
stayed enabled because it also supplies the hotspot. Offline boot and wired
DHCP with a physical Ethernet connection remain unverified.

For Ethernet, distinguish configuration from physical carrier. DHCP disabled
with a static camera subnet limits which wired network works; `NO-CARRIER`
also requires a working cable and link partner. Check the effective generated
network configuration and `networkctl status eth0` before changing managers.

## Development validation

```bash
python3 -m unittest discover -s scripts/tests -v
```

Fallback tests simulate device transitions and client timeouts. Installer tests
cover profile collisions, preserved credentials and failure recovery without
accessing the workstation's network interfaces.
