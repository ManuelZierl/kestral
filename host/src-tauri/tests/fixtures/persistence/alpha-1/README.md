# Kestral alpha.1 persistence corpus

These files freeze the byte-level `0.1.0-alpha.1` persistence baseline.
`{{PROFILE_ROOT}}` is a fixture token replaced with the materialized temporary
profile path before runtime validation. `CORPUS.json` records the SHA-256 of
the committed bytes before substitution.

The remote-owner sample contains a deterministic public test credential. It is
not a usable private authenticator. Owner sessions and ceremonies are
intentionally absent because they are process-local authority.

Run the fixture contract test after any intentional format change. Never
rewrite these files to match a later release; add a new release directory.
