<div align="center">
  <img src="host/src-tauri/icons/kestral-icon.svg" width="112" alt="Kestral logo">
  <h1>Kestral</h1>
  <p><strong>Kestral is a personal-first, open-source AI workspace and lean local host for user-chosen apps.</strong></p>
  <p>Chat is the default starting app, not the canonical interface for all AI work.</p>
  <p>
    <a href=".github/workflows/ci.yml"><img src="https://img.shields.io/badge/CI-GitHub_Actions-2088ff?logo=githubactions&amp;logoColor=white" alt="CI: GitHub Actions"></a>
    <a href="docs/alpha-release.md"><img src="https://img.shields.io/badge/status-alpha-f59e0b" alt="Project status: alpha"></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-6f9aff" alt="License: MIT"></a>
    <a href="docs/contributing.md"><img src="https://img.shields.io/badge/contributions-welcome-4db86a" alt="Contributions welcome"></a>
  </p>
  <p>
    <a href="docs/getting-started.md">Getting started</a> ·
    <a href="docs/index.md">Documentation</a> ·
    <a href="docs/curated-apps.md">Curated apps</a> ·
    <a href="docs/contributing.md">Contributing</a>
  </p>
</div>

Kestral starts with Chat so a new workspace is useful immediately, but Chat is
one ordinary app rather than the presumed interface for every task. Notes,
visual tools, model providers, agent engines, and other focused experiences stay
in userland. Capability actions routed through Kestral use the same grants,
Runs, and provenance path whether an app is bundled or installed later.

> **Status: preparing v0.1.0-alpha.1.** This planned first public testing release
> has not been published. It is for developers, early technical testers, and
> contributors. It is not
> production-ready. See the [alpha release notice](docs/alpha-release.md),
> [Getting started](docs/getting-started.md), and
> [alpha limitations](docs/honest-gaps.md).

## Quick start

Run the kernel suite or start the native desktop development host:

```sh
cargo test -p app-host-kernel

cd host
npm install
npm run tauri dev
```

## Documentation

The self-contained 0.1 series documentation lives under [`docs/`](docs/):

- [Getting started](docs/getting-started.md)
- [Using Kestral](docs/user-guide.md)
- [Architecture](docs/architecture.md)
- [Trust model](docs/trust-model.md)
- [Operations](docs/operations.md)
- [v0.1.0-alpha.1 release notice](docs/alpha-release.md)
- [Security policy](SECURITY.md)
- [Alpha limitations](docs/honest-gaps.md)
- [Contributing](docs/contributing.md)

## License

Kestral is available under the [MIT License](LICENSE).
