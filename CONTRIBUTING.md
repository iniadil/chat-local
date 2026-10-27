# Contributing to ChatLocal

Thanks for your interest. Bug reports, feature ideas and pull requests are
welcome.

## Reporting a bug

Open an issue using the bug report template. Please include your OS and
version, the ChatLocal version, and what you expected to happen. If the problem
is about discovery or connecting, say whether both machines are on the same
Wi-Fi or wired network and whether manual addresses or server mode were used.

## Development setup

See [Build from source](README.md#build-from-source) in the README for
prerequisites. In short:

```sh
npm install
npm run tauri dev
```

Before opening a pull request, run:

```sh
cargo test -p lanchat-net
cargo clippy --workspace
cargo fmt --all
npm run typecheck
```

## Pull requests

- Keep each pull request focused on one change.
- Networking and storage logic belongs in `crates/lanchat-net` and should come
  with a test.
- UI strings go through `src/i18n.ts` so both languages stay in sync.
- Describe what changed and how you tested it. Note which platforms you ran
  it on, since Windows and macOS behaviour is not covered by CI.

## Releasing

Bump the version in `package.json`, `src-tauri/tauri.conf.json`,
`src-tauri/Cargo.toml` and `crates/lanchat-net/Cargo.toml`, add a section to
`CHANGELOG.md`, then push a tag such as `v0.2.0`. The workflow builds the
installers and creates a draft release to review and publish.
