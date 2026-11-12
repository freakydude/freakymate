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
open an issue or ask on Discord, so we can agree on the approach first and your work does not go to waste.

## Pull requests

- `main` is protected: changes go through a pull request, all CI checks must be green and the code owner
  (`.github/CODEOWNERS`) has to approve it.
- Run `scripts/verify.sh` before you push (format, clippy, tests, builds, previews); for UI changes
  `scripts/verify.sh --snapshot` and look at the screenshots. See [Development](docs/development.md).
- One topic per pull request, with a short description of what changes and why.
- New dependencies need a license compatible with GPL-3.0-or-later; CI checks this with `cargo deny`
  (allowed licenses in `deny.toml`).

## Contributor License Agreement

Before your first pull request is merged, I ask you to agree to the [Contributor License Agreement](CLA.md) once
(currently a draft). You keep the copyright of your work. The agreement keeps it possible for me to offer
FreakyMate under other terms in addition to the GPL (see [COMMERCIAL.md](COMMERCIAL.md)); your contributions always
stay available under the GPL.

## License

By contributing you agree that your contributions are licensed under GPL-3.0-or-later and under the terms of the
[CLA](CLA.md).
