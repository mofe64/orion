# Wi-Fi fallback hotspot

The optional helper starts a password-protected hotspot when normal Wi-Fi fails. Clients reach SSH at `10.42.0.1` and Studio at `http://10.42.0.1:7447`.

NetworkManager must own the Wi-Fi interface. Installing the helper does not change network managers. Codex replies and web search still require internet access.

## Install on the Pi

Normal Wi-Fi must already connect automatically. The radio must support access-point mode. Run these steps from the repository root after installing Orion.

1. Install the network tools.

   ```bash
   sudo apt-get install -y dnsmasq-base iw
   ```

2. Check access-point support for the selected interface.

   ```bash
   nmcli -g WIFI-PROPERTIES.AP device show wlan0
   ```

3. Preview installation with your country's code and a unique network name.

   ```bash
   sudo /opt/orion/python/bin/python3.12 scripts/install_pi_wifi_fallback.py --country GB --ssid Orion-Lamp --plan
   ```

4. Install the helper with the same settings.

   ```bash
   sudo /opt/orion/python/bin/python3.12 scripts/install_pi_wifi_fallback.py --country GB --ssid Orion-Lamp
   ```

5. Save the printed hotspot password securely.

`--interface` selects another interface. Repeated installation preserves the password. The installer keeps the active Wi-Fi connection unchanged.

## Connection behavior

The helper leaves connected, connecting and disconnecting interfaces alone. For a disconnected interface, it first retries the saved home connection. It starts the hotspot if that attempt fails.

The hotspot stays active until an explicit connection change or reboot. Loss of internet alone does not trigger it. Local controls, timers and scenes can continue without internet.

| Path | Function |
| --- | --- |
| `/etc/orion/wifi-fallback.env` | Interface and connection IDs |
| `/usr/local/sbin/orion-wifi-fallback` | Recovery helper |
| `/etc/systemd/system/orion-wifi-fallback.timer` | Periodic checks |
| `/var/lib/orion/wifi-fallback-backups/` | Configuration backups |

NetworkManager provides address assignment through `dnsmasq-base`. The helper does not stop Orion services or change servo state.

## Connect and return home

1. Join the configured hotspot from your computer.
2. Connect to the Pi through SSH.

   ```bash
   ssh USER@10.42.0.1
   ```

3. Set Studio's address to `http://10.42.0.1:7447`.
4. Restore the home connection when it becomes available.

   ```bash
   sudo sh -c '. /etc/orion/wifi-fallback.env; nmcli --wait 20 connection up uuid "$ORION_WIFI_HOME_UUID" ifname "$ORION_WIFI_INTERFACE"'
   ```

The connection change closes hotspot SSH. Reconnect through the normal hostname afterward. The hotspot password and Studio token are separate credentials.

## Troubleshooting

1. Read network recovery messages.

   ```bash
   journalctl -u orion-wifi-fallback -u NetworkManager -n 40 --no-pager
   ```

2. Read the saved hotspot password when needed.

   ```bash
   sudo nmcli --show-secrets -g 802-11-wireless-security.psk connection show id orion-fallback-hotspot
   ```

3. Disable future recovery checks when needed.

   ```bash
   sudo systemctl disable --now orion-wifi-fallback.timer
   ```

Disabling the timer does not close an active hotspot. An explicit connection change closes it.
