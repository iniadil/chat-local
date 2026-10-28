# Changelog

All notable changes to ChatLocal are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.4.0] - 2026-10-28

### Added
- Send status on your own messages, like a phone messenger: a clock with the
  percentage while an attachment is on its way, a tick once it is sent, and
  *Not sent · Retry* under the bubble when it did not get through. Retry
  sends the same message again only to the computers that missed it. The
  progress bar above the message box now only shows files coming in.

### Fixed
- Attachments that took longer than about 25 seconds to send (large images
  or files over Wi‑Fi) failed near the end with "connection closed" on the
  receiver and an unreachable error on the sender. A keepalive ping that
  fired while a chunk was half read discarded the bytes already read, so
  the stream fell out of step and the connection dropped. Frames are now
  read whole in their own task.
- A connection whose writer failed stayed listed as online until the read
  side noticed; it now closes at once.

## [0.3.0] - 2026-10-28

### Added
- A welcome screen on first launch asks for your name or the device's name
  (prefilled with the computer name), so people recognise you when you show
  up on their network. The name can still be changed in Settings.
- *Add computer…* now finds other ChatLocal installations for you. It lists
  what mDNS/Bonjour announced and, in addition, probes every address of the
  local `/24` subnets at the ChatLocal port, which also works on networks
  that block mDNS. Each result shows the name and address with a *Connect*
  button; typing an address by hand remains available underneath. A probe
  only reads the other side's hello and never registers as a peer there.

### Changed
- Dropped, picked and pasted files no longer send immediately. They wait as
  previews above the message box (thumbnail for images) and go out when you
  press Send, so nothing leaves the machine by accident. Each item can be
  removed before sending.

### Fixed
- Ticking *Remember and reconnect automatically* in *Add computer…* no
  longer drops the connection that was just made. Changing only the manual
  addresses no longer restarts networking; new addresses are dialed right
  away.
- A manual address reached through a port forward is no longer redialed
  every few seconds, which kept replacing the working connection.
- The message box stays visible when the window is made smaller; the
  message and conversation lists scroll instead.

## [0.2.2] - 2026-10-28

### Added

- A microphone picker under *Settings → Voice notes*. The choice is kept per
  computer; if that microphone is unplugged, recording falls back to the
  system default.

## [0.2.1] - 2026-10-28

### Fixed

- Voice notes on macOS. The 0.1.1 and 0.2.0 bundles were signed with the
  hardened runtime but without the microphone entitlement, so macOS refused
  the microphone silently instead of asking for permission. The bundle now
  carries `com.apple.security.device.audio-input`, and the first click on
  the microphone shows the normal permission dialog.
- The install steps explain the macOS 15 "could not verify" dialog and the
  *Open Anyway* route.

## [0.2.0] - 2026-10-28

### Changed

- Attachments are streamed in 256 KB chunks from disk to disk instead of
  being held in memory. Memory use no longer depends on file size, the
  attachment cap rises from 512 MB to 4 GB, and the minimum RAM in the
  system requirements drops from 4 GB to 2 GB.
- Text messages and pings are written ahead of queued file chunks, so chat
  stays responsive while a large file is in transit. A relay forwards chunks
  without storing files that are not addressed to it.
- A transfer that is interrupted leaves no partial file; the receiver is told
  it failed. The UI shows progress of files being sent or received.
- **Protocol version 2.** Peers running 0.1.x cannot exchange messages with
  this version; update every computer.

## [0.1.1] - 2026-10-28

### Fixed

- macOS builds are now fully ad-hoc signed. The 0.1.0 DMGs carried only a
  linker signature, so Gatekeeper reported the app as "damaged and can't be
  opened". The app is still unsigned by an Apple developer identity, so the
  usual "unidentified developer" warning remains; see the README.

## [0.1.0] - 2026-10-28

First public release.

### Added

- Automatic discovery of other computers on the same network (mDNS/Bonjour).
- Shared "Everyone" room and private direct messages.
- Voice notes recorded as WAV.
- Image and document attachments via file picker, drag and drop, or clipboard
  paste, each verified with a SHA-256 hash.
- Local message history and received files that survive restarts.
- Optional server (relay) mode for networks where peers cannot see each other
  directly.
- Manual peer addresses for networks without mDNS.
- Indonesian and English interface, light and dark themes.
- Installers for Windows (NSIS, MSI) and macOS (Apple Silicon, Intel) built by
  GitHub Actions.

### Known limits

- Traffic is not encrypted; use only on a network you trust.
- Builds are not code-signed, so Windows and macOS show a warning on first run.
- Attachments are held in memory during transfer and capped at 512 MB.
- Messages to an offline computer are rejected rather than queued.
- Not yet verified on physical Windows or macOS hardware; see the README.

[0.4.0]: https://github.com/iniadil/chat-local/releases/tag/v0.4.0
[0.3.0]: https://github.com/iniadil/chat-local/releases/tag/v0.3.0
[0.2.2]: https://github.com/iniadil/chat-local/releases/tag/v0.2.2
[0.2.1]: https://github.com/iniadil/chat-local/releases/tag/v0.2.1
[0.2.0]: https://github.com/iniadil/chat-local/releases/tag/v0.2.0
[0.1.1]: https://github.com/iniadil/chat-local/releases/tag/v0.1.1
[0.1.0]: https://github.com/iniadil/chat-local/releases/tag/v0.1.0
