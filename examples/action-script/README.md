# Restricted action scripts (prototype)

**Status: experimental userland runtime adapter, not a shipped/installable app.**
This is the parser + host-action-path proof of concept for [issue #16](https://github.com/ManuelZierl/kestral/issues/16).
It intentionally does **not** change `crates/kernel`, introduce a workflow engine,
or execute arbitrary Rhai. It uses Rhai **only for parsing**. The code lives
outside the root Cargo workspace so its exactly pinned Rhai frontend can be
evaluated without adding it to the trusted core's dependencies.

## Grammar: `kestral-action-1`

```rhai
let person = contacts.lookup("Anna");
let receipt = directory.mask(person.iban);
return receipt;
```

Or a nested call (compiled into the same two sequential steps):

```rhai
return directory.mask(contacts.lookup("Anna").iban);
```

Allowed:
- `let name = literal | capability_call | previous_result | previous_result.field`;
- UTF-8 string, 64-bit integer, boolean and `()`/null literals;
- static `provider.action(arg, ...)` bindings; calls may be nested;
- direct fields on previously returned objects with *required, typed* output
  schema properties; a final explicit `return expression;`;
- plain capability-call statements whose result is intentionally discarded.

Rejected: all other syntax, including assignment, loops, branching, functions,
closures, arithmetic, imports, dynamic dispatch, indexing, native Rhai
functions, interpolated strings, and property mutation. No `eval_ast` or
`Engine::eval` is used.

The **host** supplies an explicit `BindingSpec` map:
`provider.action` alias → canonical `CapabilityRef`, positional JSON property
names, trusted effect policy, and coarse sensitivity flags. Generated scripts
cannot choose grants, change argument mappings, or use arbitrary registry tools.
A binding requires an installed provider with a compatible declared schema.
Any nontrivial JSON Schema construct not covered by the MVP's static checking
remains subject to the kernel's full input/output JSON Schema validation.

`CompiledProgram::inspect()` returns a stable, serializable
`kestral-action-1` plan containing canonical capability identities and typed
reference expressions. It is for inspection, **not an authorization token**.

## Run and approval behavior

`Session::start` opens a real Run under the installed consuming app.
`prepare_next` resolves only the next step's arguments using immutable prior
results, then calls `Kernel::prepare_invocation`. `resume` consumes that
single-use prepared token, calls `await_approval` in the **host-owned** trusted
chrome, `authorize_invocation`, then executes and finalizes the capability.
The kernel validates the grant, schema, cancellation, and provider identity,
writes approval/attempt/result events, and stamps artifact provenance.

The session deliberately pauses **before** trusted approval, and remembers
completed results in-process. Calling `prepare_next` again on a pending
step does not redo work; calling `resume` without a pending step fails. Denied
approvals stop the run without performing the denied call. Approval is bound
by the kernel to the **resolved JSON arguments**, not to source text.

For non-verified capabilities, `BindingPolicy::RequiresApproval` rejects
a silent/notify grant that might bypass trusted chrome. An imported MCP tool's
`readOnlyHint`, declared `effect`, or generated description is **not evidence**
that it is read-only. `VerifiedReadOnly` is reserved for an operator-verified
wrapper with genuinely constrained credentials/behavior.

`sensitive_output` / `accepts_sensitive` implement a conservative **single
result-flow guard**, not complete information-flow control. A lookup result
cannot be passed to an unapproved sink unless the host explicitly enables it.
Sensitive values can still be present in user literals or returned to the app;
do not treat this flag as DLP.

## Budgets

| Limit | Prototype value |
| --- | --- |
| Source UTF-8 | 8 KiB |
| Parsed AST nodes | 256 |
| Static capability steps | 16 |
| Arguments per step | 16 |
| Concurrent capability calls per script | 1 |
| Whole-session wall time | 120 seconds |
| Stored result per step | 64 KiB |
| Sum of stored results | 256 KiB |
| Loops/recursion | None |

This is an **in-process prototype**. Hard process memory limits and crash
containment require a sandboxed worker. The kernel's per-invocation timeout
and cooperative cancellation still apply. It cannot force a remote tool to
roll back a side effect already performed.

## Run tests

```sh
cargo test --manifest-path examples/action-script/Cargo.toml
cargo fmt --manifest-path examples/action-script/Cargo.toml -- --check
```

Tests install ordinary provider/consumer app manifests into the real kernel,
including a two-capability chain, provenance, exact-argument approvals,
deny/no-write, a one-time mock write, malformed scripts, static types,
budgets, a simulated unavailable MCP tool, and partial completion.

## Three separate guarantees

1. **Script correctness:** a restricted parser and plan compiler reject
   unsupported code and known type errors *before effects*. Runtime fields
   and output size may still fail after prior steps have completed.
2. **Capability permission:** only the kernel's grants and trusted approval
   authorize effects. The script language itself confers **zero** authority.
3. **Real-world reversibility:** not provided. A successful external write
   may be irreversible. The runtime never retries an effect automatically,
   claims a transaction rollback, or treats preparation as read-only without
   evidence.

## Remaining work before a production Kestral app

- Package this userland adapter into an installable app with a restricted
  broker-mediated host RPC/MCP transport, *without passing a raw Kernel handle
  to untrusted external code*. The current `Kernel` integration is a host
  adapter spike/test harness only.
- Preserve approvals/resume tokens through an async host executor. Today's
  session is in-memory; crash recovery marks incomplete Runs interrupted,
  but does not resume external effects. A durable resumable design must have
  operation IDs, result checkpoints, and idempotency handling.
- Enforce fine-grained resource data scopes, robust taint tracking, schema
  unions, cancellation propagation across an app process, and deliberate
  human-in-the-loop prompts for ambiguous/partial data.
- Benchmark the grammar against direct tool-calling and shell generation on
  the same ~1.7B local model and tasks.

**Do not attach live payment, PostgreSQL-write, or broad HTTP tools to this
prototype.** Use mock providers or trusted read-only wrappers for the spike.
