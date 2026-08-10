# Portable workspace v1 fixtures

This directory contains format fixtures only. It must never contain credential
values, OS-vault exports, remote-owner passkeys, or third-party app binaries.

`empty/kestral-portable.json` represents a valid manifest-first archive with no
content entries. Archive corruption and round-trip behavior are generated in
temporary directories by `portable/tests.rs` so tests never write fixtures.
