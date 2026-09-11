# {{APP_NAME}}

`{{APP_ID}}` is a small, backend-free Kestral app generated from the focused-app
scaffold. It demonstrates the shortest useful path: a custom interface, durable
host-managed data, and an optional model action whose result remains a draft
until the person adds it.

No dependency installation is needed. Node.js 22 or newer is sufficient.

```bash
npm test
npm run build
```

Then open **Apps → Install an app** in Kestral and select this project's `dist`
directory. Re-run `npm run build` after editing `src/app.json` or
`src/ui/index.html`; set the release version in `package.json`. The build swaps
in a complete `dist` directory and pins every UI asset hash.

The generated surface has no direct network access and no native backend. Its
only requested authority is `llm-provider/llm.generate`, with approval required
for each use. The list continues to work if that grant is denied or no model is
configured. If you intentionally add a backend, network origin, capability, or
broader grant, update the safety assertions in `test/package.test.mjs` so that
the authority change is explicit in review.

The package contract is documented in Kestral's `docs/writing-apps.md`. Kestral
performs the authoritative schema and semantic validation during package
inspection; this project's tests cover its intended shape and integrity.
