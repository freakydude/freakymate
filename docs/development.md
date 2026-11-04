# Development

Rust stable (see `rust-toolchain.toml`). On Linux install `libfontconfig1-dev libxkbcommon-dev libinput-dev libudev-dev libgbm-dev`
(the last three for the framebuffer fallback, see [install.md](install.md)).
For Windows without admin rights (setup, faster linking with `rust-lld`, build times) see
[windows-without-admin.md](windows-without-admin.md). Structure, data flow and conventions:
[architecture.md](architecture.md). How pull requests are handled: [CONTRIBUTING.md](../CONTRIBUTING.md).

## Try it without a printer

The `fake-moonraker` crate is a simulated Moonraker/Klipper (WebSocket JSON-RPC, files, thumbnails, heaters, a print job
that progresses). It listens on `127.0.0.1` only.

```bash
cargo run -p fake-moonraker -- --profile voron --port 7125 --time-scale 10   # the simulated printer
cargo run -p freakymate -- -m http://127.0.0.1:7125                       # the app, in a window
```

| Fake option    | Meaning                                                                          |
| -------------- | -------------------------------------------------------------------------------- |
| `--profile`    | `basic`, `voron` (default), `happy-hare` or `ercf`, see below                    |
| `--gates`      | number of MMU gates, 1 to 16 (default 4; only with `happy-hare` and `ercf`)      |
| `--port`       | port to listen on (default 7125; only one fake can run per port)                 |
| `--time-scale` | simulation speed (default 10): heating and printing run that many times faster   |
| `--auth`       | `<user>:<password>`: Moonraker wants a login (also accepts the API key `fake-api-key`) |

| Profile      | Simulates                                                                                                                  |
| ------------ | -------------------------------------------------------------------------------------------------------------------------- |
| `basic`      | a plain printer: nozzle, bed, fans, no optional modules                                                                    |
| `voron`      | adds Quad Gantry Level, a chamber heater, filament sensors, `exclude_object` and Spoolman                                  |
| `happy-hare` | adds a Happy Hare MMU with a proportional sync-feedback buffer and FlowGuard, spools in the gates, and Spoolman            |
| `ercf`       | like `happy-hare`, but without Spoolman: material and colour of the gates live in Happy Hare's gate map                    |

`FAKE_MMU_TANGLE BOOST=1` (G-code, simulator only) lets tangle prevention boost the gear current during a print. In VS Code
the tasks and debug configurations "run: fake-moonraker", "run: freakymate (fake Moonraker)" and their Happy Hare
variants start both. The app also takes `--simulate-network` in development builds to try the Wi-Fi settings without
NetworkManager.

## Checks, screenshots and previews

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
scripts/verify.sh [--snapshot]                           # WSL2/Arch: all of the above plus previews and the skill check in one go
scripts/ui-preview.sh                                    # renders crates/ui/ui/preview/*.slint with sample data in seconds
scripts/ui-snapshot.sh [-p happy-hare] [-s 1024x600] [-F] [-S pages,printing]   # starts the fake itself, renders the screens to PNG
cargo run -p ui-snapshot -- --out shots --scenes all     # the same by hand (needs the fake running)
cargo run -p icon-index                                  # after changing icons in assets/icons
```

`ui-snapshot` runs the real UI headless against the fake and writes one PNG per screen (`-p` picks the fake profile,
`-s` the window size, `-F` switches the fitting to the display off, `-S` chooses scenes: `pages`, `spools`, `network`,
`taps`, `typing`, `calibrate`, `printing`, `shutdown`, and only when chosen `idle` (slow) and `login` (the fake then wants
a login)). `ui-preview` renders single `.slint` files with fixed sample data, no build needed.

Scripts exist in two variants with the same behaviour:

- `scripts/*.sh` (bash) for Arch Linux, native or in WSL2, the main development environment. `scripts/setup-linux.sh` checks the environment.
- `scripts/*.ps1` (PowerShell) for Windows without admin rights. `scripts\setup.ps1` prepares the GNU toolchain there, see [windows-without-admin.md](windows-without-admin.md).

## Tools

| Tool                                                                                   | Needed for                                      | How to get it                                                                                                           | Where it is looked up                                                                  |
| -------------------------------------------------------------------------------------- | ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Rust (`rustup`)                                                                        | everything                                      | [rustup.rs](https://rustup.rs); `rust-toolchain.toml` selects the version                                               | `PATH`                                                                                 |
| MinGW linker ([w64devkit](https://github.com/skeeto/w64devkit))                        | building on Windows without MSVC                | `scripts\setup.ps1` downloads it                                                                                        | `%USERPROFILE%\tools\w64devkit\bin`, added to the user `PATH` by the script            |
| [`slint-viewer`](https://docs.slint.dev/latest/docs/slint/guide/tooling/slint-viewer/) | `scripts/ui-preview.sh` (WSL2), `scripts\ui-preview.ps1` (Windows) | WSL2: the script compiles the version matching `slint` in `Cargo.lock` into `~/.local/bin` (`cargo install slint-viewer --version <same>`); Windows: the script downloads it | `slint-viewer` on `PATH` first, otherwise the script's install location |
| Slint extension for VS Code (optional)                                                 | live preview of `crates/ui/ui/preview/*.slint` | VS Code marketplace (`Slint.slint`)                                                                                     | –                                                                                      |

To use your own `slint-viewer`, put it on `PATH`; it should have the same version as the `slint` crate.

## Faster builds (optional, Linux)

[`sccache`](https://github.com/mozilla/sccache) caches compiled crates across `cargo clean`, branches and checkouts
on Arch Linux, native or in WSL2. It skips incremental builds, so it mainly saves rebuilding the dependencies (Slint
above all). It is configured per user, not in the repo, so CI and machines without it are unaffected;
`scripts/setup-linux.sh` reports whether it is set up.

```bash
sudo pacman -S --needed sccache
```

`~/.cargo/config.toml`:

```toml
[build]
rustc-wrapper = "sccache"
```

`sccache --show-stats` shows the cache hits. In WSL2, keep the repo in the Linux file system (e.g. under `~`), not under `/mnt/c`.

## Development builds and releases

Local builds of `freakymate` include the cargo feature `dev-mode` (default): the option `--simulate-network`
and the simulated networks in the core. Releases are built with `--no-default-features` and contain none of it.
The development tools are separate programs and never shipped: `fake-moonraker` (simulated printer) and
`ui-snapshot` (screenshots of the UI).

## CI, versions and releases

- Versions come from git tags (`.github/scripts/version.sh`): releases are `vX.Y.0`, every later commit on `main` adds
  one to the patch number (`0.2.1`, `0.2.2`, ...). Pull request builds are `X.Y.Z-pr.<number>.<run>`, other branches
  `X.Y.Z-dev.<commit>` (started manually), local builds `0.0.0-dev`. The version is shown on System → Display and by
  `--version`.
- Every push to `main` and every pull request: format, clippy, tests, builds and packages for all targets
  (`.github/workflows/ci.yml`), with cargo caching; a branch without a pull request can be built with **Run workflow**.
  A newer commit cancels the running build of the same branch or pull request. The packages are attached to the run as artifact `packages-<version>`; pull requests get a
  comment with the download link, so an open pull request can be tried before it is merged.
- Every push to `main` is published as prerelease `vX.Y.Z`, replacing the previous prerelease. Moonraker offers it on
  the `beta` channel.
- Release: run the workflow **Release** (Actions tab) on `main` and choose `minor` or `major`. It tags `vX.Y.0`,
  builds and publishes the release with generated notes; Moonraker offers it on the `stable` channel. The patch number
  then starts again at 0.
- Assets: one zip per target and one `.deb` per Linux target (`.github/scripts/build-deb.sh`, files in
  `scripts/debian`). Linux zips contain `freakymate`, `install.sh`, `uninstall.sh`,
  `system-dependencies.json` and `release_info.json` (version and asset name for Moonraker); Windows: `freakymate.exe`.
  Every zip and `.deb` also contains `LICENSE` and `THIRD-PARTY-LICENSES.html` (licenses of all compiled-in crates,
  generated by `cargo about` from `about.toml` and `about.hbs`).
- Licenses of dependencies: `cargo deny check licenses` in CI fails on any crate whose license is not in the allow
  list of `deny.toml` (all compatible with GPL-3.0-or-later).
