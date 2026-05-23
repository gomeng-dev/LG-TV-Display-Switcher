# Changelog

All notable changes to this project are documented here.

Release notes are generated from the section matching the tag name, such as
`v0.1.3` or `0.1.3`. If that section does not exist yet, the release workflow
uses `Unreleased`.

## [Unreleased]

### Added

- Add a companion Stream Deck plugin with TV power, TV mode, PC mode, and auto-switch actions.
- Add a Stream Deck JSON CLI interface to the Windows app for companion plugin control.

### Changed

- Include the packaged `.streamDeckPlugin` in GitHub release artifacts.
- Package the Stream Deck plugin with SDK v3 compatibility for Marketplace DRM processing.

## [v0.1.4]

### Added

- Add a tray menu toggle to run the app at user startup.

## [v0.1.3]

### Added

- Add a Windows application, tray, installer, and uninstaller icon generated from `assets/app-icon.svg`.
- Store Windows audio endpoint IDs during onboarding so TV audio switching can target the exact device instead of relying only on friendly names.
- Remember the PC audio output immediately before switching to TV mode, then restore that output when PC mode is applied.
- Add `Apply TV mode` to the tray menu when the TV is not currently active.
- Add `PcAudioEndpointId`, `PcAudioDeviceNameContains`, and `TvAudioEndpointId` configuration keys.

### Changed

- Only apply TV mode when the TV is confirmed to be on.
- Use the webOS power-state endpoint when a client key is available, so standby network availability is not mistaken for the TV being on.
- Apply PC mode immediately after a successful app-initiated TV power-off command.
- Pass audio device selection data to PowerShell through environment variables so names with spaces and parentheses are handled correctly.
- Make GitHub release notes read from this changelog and include a comparison link to the previous tag.

### Fixed

- Fix the Core Audio `IMMDeviceCollection` COM GUID so active audio endpoint enumeration works reliably.
- Avoid overwriting the remembered PC audio output when the current output is already the TV audio device.
