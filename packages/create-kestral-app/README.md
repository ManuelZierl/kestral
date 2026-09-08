# create-kestral-app

Dependency-free creator tool for a standalone Kestral focused app.

```bash
npx create-kestral-app ../my-focus-app \
  --id com.example.my-focus-app \
  --name "My Focus App"

cd ../my-focus-app
npm test
```

The generated project owns its source, build, tests, and installable `dist/` package. It does not import host or kernel source and it has no runtime npm dependencies.

This package intentionally scaffolds a backend-free app first. Add native or MCP backends only when the workflow genuinely needs them, because backend authority changes the trust boundary.

## Release qualification

`npm test` creates a project in a temporary directory, rebuilds it, runs its own tests, and checks that package output is reproducible. CI additionally runs `npm pack` so the creator path is tested as a publishable package rather than only as a repository helper.
