# pi-ai Provider Worker

This worker is a private, trusted process boundary of Kestral. It is bundled with the host and is not an ordinary third-party app, plugin, or sandboxed surface. Only the host should launch it.

## Runtime and build

Production uses a self-contained Node.js v22.19.0 runtime, matching `@earendil-works/pi-ai@0.80.7`. Build and test from `host/`:

```sh
npm run provider-worker:runtime
npm run provider-worker:package
npm run provider-worker:check
npm run provider-worker:test
```

`provider-worker:runtime` supports native `win32`, `linux`, and `darwin` builds on `x64` and `arm64`. It downloads the official archive and `SHASUMS256.txt` from `nodejs.org`, requires the archive's exact checksum entry, verifies SHA-256 before extraction, and rejects every other platform/architecture. It never falls back to a system Node executable. A valid installation is cached using `runtime/install-metadata.json`; the cached executable and license hashes and the runtime's reported version are checked before reuse.

`provider-worker:package` installs the runtime and bundles the worker. The generated files are not source-controlled:

`provider-worker:test` also launches Node's test runner through that exact
bundled runtime; the developer's `node` on `PATH` is used only to bootstrap the
checksum-verified runtime installer.

```text
provider-worker/
|- dist/worker.mjs
`- runtime/
   |- node.exe       # Windows, or node on Linux/macOS
   |- LICENSE        # Node.js license
   `- install-metadata.json
```

Tauri packages only the executable, license, and worker under its resource directory. `node_modules` and the installation metadata are not bundled:

```text
$RESOURCE/provider-worker/
|- worker.mjs
`- runtime/
   |- node.exe       # Windows, or node on Linux/macOS
   `- LICENSE
```

## Private protocol

Input and output are newline-delimited JSON on stdin and stdout. Diagnostics go to stderr. The worker emits `{"type":"ready","protocol_version":2}` after startup.

Every command has a non-empty `request_id` and one of these discriminants:

- `generate`: `provider`, `model`, `messages`, optional `tools`, `reasoning`, `text_verbosity`, `temperature`, and `max_output_tokens`
- `models-list`: `provider`
- `models-refresh`: `provider`
- `oauth-login`: OAuth provider configuration `{kind, base_url?}`
- `oauth-prompt-response`: `target_request_id`, `prompt_id`, and exactly one of `value` or `cancelled: true`
- `cancel`: `target_request_id`
- `shutdown`: no additional fields

Generation and model provider configuration is `{kind, base_url?, api_key?, oauth_credential?, env?}`. `api_key` and `oauth_credential` are mutually exclusive. OAuth login deliberately accepts only `{kind, base_url?}`. Supported kinds are `ollama`, `open-ai-compatible`, `openai`, `openai-codex`, `anthropic`, `github-copilot`, `openrouter`, `google`, `mistral`, and `amazon-bedrock`. OpenAI Codex and GitHub Copilot are built-in generation/model providers. Anthropic supports either its existing API-key path or OAuth. Unknown fields and malformed nested values produce a `failed` response rather than terminating the worker.

`openai-codex` is pi-ai's ChatGPT Plus/Pro subscription provider. Its fixed base URL is `https://chatgpt.com/backend-api`; it is not the API-key-backed `openai` provider at `https://api.openai.com/v1`. Its static GPT-5.x Codex catalog can be listed without a credential, while generation and credential refresh require the OAuth credential returned by ChatGPT login. Browser login uses a loopback callback server; device-code login supports headless and split hosts.

Codex Responses models advertise `text_verbosity` values `low`, `medium`, and
`high`. The worker writes the selected value to `text.verbosity` in the final
provider payload. Other adapters advertise no values and reject the control
instead of dropping it.

Generation emits zero or more `stream-delta` records, then one of these terminal records:

```text
completed {request_id, response, credential?}
failed    {request_id, code, message, credential?}
```

`credential` is present only when request-scoped OAuth resolution rotated the input credential. If refresh succeeds and generation later fails or is cancelled, the rotated credential is returned on `failed` so the trusted host can persist it. Stream records and failure message text never contain credentials.

Protocol v2 completion usage preserves provider-reported `cache_read_tokens`
and `cache_write_tokens` separately from aggregate prompt tokens. Its
`provider_metrics` records integer request-to-first-token and total latency in
milliseconds using the worker's monotonic clock. First-token latency is absent
when a provider returns no non-empty stream delta.

Model commands emit `models {request_id, models, credential?}`. When an OAuth credential is supplied, the worker resolves model auth through pi-ai and returns any refresh on `credential`; a later model-command failure likewise returns the rotated credential on `failed`. Cancel and shutdown commands emit `acknowledged`. A cancelled generation emits `failed` with code `cancelled`.

OAuth login uses only the selected built-in pi-ai provider's `provider.auth.oauth.login` adapter. A provider without that adapter fails the request. The host drives interaction through these records:

```text
oauth-event     {request_id, event}
  auth_url      {type, url, instructions?}
  device_code   {type, user_code, verification_uri, interval_seconds?, expires_in_seconds?}
  progress      {type, message}
oauth-prompt    {request_id, prompt_id, prompt}
  text|secret|manual_code {type, message, placeholder?}
  select        {type, message, options:[{id,label,description?}]}
oauth-completed {request_id, credential}
```

`credential` and `oauth_credential` use pi-ai's `OAuthCredential`: `type: "oauth"`, non-empty `access` and `refresh` strings, a finite non-negative `expires`, and JSON-safe provider-specific fields required by adapters such as OpenAI Codex or GitHub Copilot metadata. Credential JSON has bounded total size, nesting depth, node count, object/array width, keys, token strings, and extra strings. Cycles, accessors, sparse/custom arrays, non-plain objects, non-finite numbers, symbols, and prototype-pollution keys (`__proto__`, `prototype`, `constructor`) are rejected. Prompt responses must match both the active login request and prompt ID. Select responses must be one of the advertised option IDs. Prompt values, provider text, URLs, options, tokens, and the complete credential JSON are bounded; malformed values fail instead of being forwarded.

## Credential boundary

Credentials are accepted only in each generation/model command's provider configuration. API keys remain explicit request options. OAuth credentials initialize a request-scoped `CredentialStore` for the selected provider; pi-ai resolves and refreshes through that store. Store modifications are serialized and retain only the latest credential in memory for the life of that request. The worker returns a changed credential only on `oauth-completed` or the terminal `completed`, `models`, or `failed` record. Credential strings, including input and rotated access/refresh values, are included in request-local error redaction.

Before accepting commands, the worker clears its inherited environment and injects an `AuthContext` that cannot read environment variables or files. It has no persistent credential store and does not persist keys, OAuth credentials, prompt responses, or provider environment values. The worker does not open authorization URLs and does not read from a terminal; the host owns all browser and user interaction. Concurrent logins have independent abort controllers and prompt maps. `cancel` aborts the target login and rejects its active prompts, while `shutdown` aborts all active logins and generations. Bedrock requires an explicit bearer key, static access keys in `env`, or `AWS_BEDROCK_SKIP_AUTH=1`; profile and file-backed AWS credential settings are rejected. stderr diagnostics contain only error categories; protocol error messages are bounded and redact credential-like tokens. The launcher must still avoid placing credentials in command-line arguments or inherited diagnostics.
