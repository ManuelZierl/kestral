# create-kestral-app

Dependency-free creator tool for a standalone Kestral focused app.

Requires Node.js 22 or newer. Registry publication is a release step. To use the
current source package, run `npm pack` in this directory and install the resulting
tarball with `npm install --global ./create-kestral-app-0.1.0-alpha.1.tgz`.
Then, from your projects directory:

```bash
create-kestral-app my-focus-app \
  --id com.example.my-focus-app \
  --name "My Focus App"

cd my-focus-app
npm test
```

The generated project owns its source, build, tests, and installable `dist/` package. It does not import host or kernel source and it has no runtime npm dependencies.

This package intentionally scaffolds a backend-free app first. Add native or MCP backends only when the workflow genuinely needs them, because backend authority changes the trust boundary.

## Release qualification

`npm test` packs the actual tarball, installs it in a temporary directory, and
invokes its public npm binary to create a project. It rebuilds the project, runs
its tests, and checks output reproducibility. Both CI platforms exercise this
path, including directories with spaces.
