# Changelog

All notable changes to ChatLocal are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

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

[0.1.0]: https://github.com/iniadil/chat-local/releases/tag/v0.1.0
