# LG TV Display Switcher Installation Guide

This guide explains how to install and set up the `LG-TV-Display-Switcher` Windows app and Stream Deck plugin.

Korean version: [INSTALLATION.md](INSTALLATION.md)

## Requirements

- Windows 10 or later
- Your LG webOS TV and PC must be connected to the same local network.
- To turn the TV on from the PC, you need the TV MAC address and Wake-on-LAN support enabled on the TV.
- Elgato Stream Deck 7.1 or later is recommended for the Stream Deck plugin.

## Install The Windows App

1. Open the latest GitHub release page.
   - https://github.com/gomeng-dev/LG-TV-Display-Switcher/releases/latest
2. Download `LG-TV-Display-Switcher-Setup.exe`.
3. Run the installer.
4. If the DisplayConfig PowerShell module is missing, the installer will install it as a dependency.
5. After installation, launch `LG-TV-Display-Switcher`.

Windows SmartScreen or your browser may show a warning. Personal distribution apps can trigger warnings until the code signing reputation has built up enough trust.

## First-Run Onboarding

1. On first launch, the app scans your local network for LG TVs.
2. Select the discovered TV and approve the pairing prompt shown on the TV screen.
3. If the TV is connected to the PC through HDMI, apply TV mode.
4. Review the audio device list and confirm the TV audio endpoint.
   - Example: `LG TV SSCR2 (NVIDIA High Definition Audio)`
5. After onboarding completes, the app keeps running in the Windows tray.

## Tray Menu

Right-click the tray icon to access these features.

- `Apply TV Mode`: Switches to the TV display mode when the TV is on.
- `Apply PC Mode`: Switches back to the PC display mode.
- `TV Power`: Turns the TV on or off.
- `Auto Switch Displays`: Automatically switches between TV and PC modes when the TV power state changes.
- `Run as Startup`: Starts the app automatically when you sign in to Windows.

When returning to PC mode, audio is restored to the default output device that was active immediately before entering TV mode.

## Install The Stream Deck Plugin

1. Download `dev.gomeng.lg-tv-display-switcher.streamDeckPlugin` from the latest GitHub release page.
2. Double-click the file.
3. Approve the installation in the Stream Deck app.
4. Find the `LG TV Display Switcher` category in the Stream Deck action list.
5. Drag the actions you want onto your Stream Deck buttons.

Available actions:

- `TV Power Toggle`: Toggles TV power.
- `Display Mode Switch`: Toggles between PC mode and TV mode. TV mode only works when the TV is on.
- `Toggle Auto Switch`: Turns automatic display switching on or off.

The Stream Deck plugin does not include the Windows app. It calls the installed `LG-TV-Display-Switcher.exe` as a companion app. If the Windows app is not installed, the button shows `Install app` or `App missing` and opens the latest GitHub release page.

## Configuration Files

App configuration is stored under your local app data directory. In most cases, you can find it here:

```text
%LOCALAPPDATA%\LG-TV-Display-Switcher\
```

Important settings:

- `TvHost`: LG TV IP address or host name
- `TvMac`: TV MAC address used for Wake-on-LAN
- `AutoSwitchDisplays`: Whether display mode should change automatically when TV power changes
- `AutoSwitchAudio`: Whether the default audio output should change with TV/PC mode

## Troubleshooting

### The TV Is Not Discovered

- Make sure the PC and TV are on the same network.
- Assign a static IP address to the TV or use DHCP reservation on your router.
- Enter the TV IP address directly in `TvHost`.
- Check whether Windows Firewall is blocking local network traffic.

### Turning The TV On Does Not Work

- Enable network standby features on the TV, such as Wake-on-LAN, Quick Start, or Mobile TV On.
- Make sure `TvMac` is correct.
- Wired LAN is usually more reliable than Wi-Fi for Wake-on-LAN.

### TV Mode Does Not Apply

- Make sure the TV is actually turned on.
- Check whether Windows detects the TV as a display.
- Check the HDMI cable and GPU output port.
- `Apply TV Mode` is designed to fail when the TV is off.

### Audio Does Not Switch To The TV

- Check whether the TV audio device appears in Windows sound settings.
- The TV audio endpoint may appear a few seconds after entering TV mode, so try again after a short delay.
- Confirm that the TV audio name shown during onboarding matches the actual Windows audio device name.

### Stream Deck Shows `App missing`

- Install the Windows app first.
- Restart the Stream Deck app after installation.
- The plugin searches for the app in this order:
  - `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\LG-TV-Display-Switcher` `InstallLocation`
  - `%LOCALAPPDATA%\LG-TV-Display-Switcher\LG-TV-Display-Switcher.exe`

## Uninstall

1. Remove `LG-TV-Display-Switcher` from the Windows Apps list.
2. Remove the `LG TV Display Switcher` plugin from the Stream Deck app.
3. If needed, delete the `%LOCALAPPDATA%\LG-TV-Display-Switcher\` configuration folder.
