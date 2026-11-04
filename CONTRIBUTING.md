# Contributing to Bulk

Thanks for wanting to help. Bug reports, ideas and pull requests are all welcome.

## Reporting a bug

Open an issue and include:

- your Windows version,
- whether fast mode was on (Settings → Fast mode),
- what you did, what you expected, and what happened instead,
- anything relevant from `%LOCALAPPDATA%\Bulk\helper.log`.

Found a security problem? Please don't open a public issue; see [SECURITY.md](SECURITY.md).

## Suggesting a feature

Open an issue first and describe the problem you're trying to solve. For anything
bigger than a small fix, it's worth agreeing on the approach before writing code.

## Setting up

You need Windows 10 or 11, Rust (stable, MSVC toolchain) with the Visual Studio C++
build tools, and Node 20+ with pnpm.

```bash
pnpm install
pnpm tauri dev
```

The first run asks for elevation once, to register the dev helper. The
[Developing](README.md#developing) section of the README covers the CLI, the
benchmark tools and a few gotchas.

## Before you open a pull request

Run the checks and make sure they pass:

```bash
cargo test -p bulk-core -p bulk-cli
cargo clippy --workspace
pnpm check
pnpm format
```

Then:

- keep each pull request to one change,
- add or update tests for scanning, search and filter logic in `bulk-core`,
- update the README if you change how something works or add a feature,
- describe what you changed and how you tested it.

## Code that deletes files or runs elevated

Bulk deletes files and runs a helper with admin rights, so changes there get a
closer look. Read the [Security model](README.md#security-model) first. In
particular, the helper must stay read-only, and deletes must keep running in the
normal app or CLI with the user's own rights.

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE), the same as the rest of the project.
