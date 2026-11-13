# Contributing

Bug reports, ideas and pull requests are welcome. FreakyMate is a hobby project that I maintain in my spare time,
so replies may take a while. Questions and discussions fit best on [Discord](https://discord.gg/Ja2cxCcmJQ), bugs
and feature requests in the GitHub issues.

## Bug reports

Please use the issue forms and search the existing issues first. A bug report needs the diagnostic report
(`freakymate --report`) and the verbose log; [Bug reports](docs/configuration.md#bug-reports) explains how to get
both. Issues in German are welcome too.

## How the project is run

I maintain FreakyMate and decide what goes in and where it is heading. Before you start on a bigger change, please
open an issue or ask on Discord, so we can agree on the approach first and your work does not go to waste. Small
fixes (typos, an obvious bug) can go straight to a pull request.

A few principles shape every change:

- **Show facts, not guesses.** The screen shows what Klipper, Moonraker and the plugins report. Macros are black
  boxes, called by name.
- **Hints, not locks.** Where something looks unusual (wrong filament, a fan that stopped), the screen points it out
  and lets the user decide.
- **Only what the printer has.** A function without its hardware or plugin does not appear at all.
- **Domain logic in the core** (`crates/core`), the UI (`crates/ui`) only shows it; [Architecture](docs/architecture.md)
  explains the structure.

## Your first pull request

1. **Fork** the repository on GitHub and clone your fork.
2. **Set up** Rust with [rustup](https://rustup.rs) (`rust-toolchain.toml` picks the version) and the system libraries
   from [Development](docs/development.md). Development happens on Linux; `scripts/setup-linux.sh` checks what is
   missing.
3. **Branch** off `main`: `git switch -c fix/short-name`.
4. **Try it without a printer**: start the simulated printer and the app (see
   [Try it without a printer](docs/development.md#try-it-without-a-printer)). There are profiles for a plain
   printer, a Voron, Happy Hare, AFC and tool changers.
5. **Check** before you push: `scripts/verify.sh` runs format, Clippy, tests, builds and the UI previews in one go.
   For UI changes run `scripts/verify.sh --snapshot` and look at the screenshots it writes.
6. **Open the pull request** against `main`: one topic per pull request, with a short description of what changes,
   why, and how you tested it (simulator profile, real printer). Screenshots help for UI changes.
7. **CI** runs the same checks for all targets and builds the packages. They are listed under Artifacts on the page
   of the CI run (open the checks of the pull request), so you and others can try the change on a printer before it
   is merged.
8. **Review**: the code owner (`.github/CODEOWNERS`) has to approve, and all checks must be green; `main` is
   protected. I may ask for changes; push them to the same branch.

## Rules for code

- Format with `cargo fmt`, no Clippy warnings (`-D warnings`), tests for logic in the core.
- Comments, names, log and error texts in English.
- New dependencies need a license compatible with GPL-3.0-or-later; CI checks this with `cargo deny` (allowed
  licenses in `deny.toml`). Please mention a new dependency in the pull request.
- Changes that need a plugin you do not have: extend the simulated printer (`fake-moonraker`) from the plugin's
  source code and name the source (file and commit) next to the code.

## Contributor License Agreement

Before your first pull request is merged, I ask you to agree to the [Contributor License Agreement](CLA.md) once
(currently a draft). You keep the copyright of your work. The agreement keeps it possible for me to offer
FreakyMate under other terms in addition to the GPL (see [COMMERCIAL.md](COMMERCIAL.md)); your contributions always
stay available under the GPL.

## License

By contributing you agree that your contributions are licensed under GPL-3.0-or-later and under the terms of the
[CLA](CLA.md).
