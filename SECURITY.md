# Security Policy

## Supported versions

Kestral is preparing its first public release. Before `v0.1.0-alpha.1` is
published, no development revision is supported as a stable security baseline.
After publication, only the latest public prerelease is supported while the
project remains in alpha.

Alpha support means confirmed vulnerabilities will be assessed and fixed when
practical. It does not imply production readiness, a response-time guarantee,
or a bug bounty.

## Report a vulnerability

Use [GitHub private vulnerability reporting](https://github.com/ManuelZierl/kestral/security/advisories/new)
for suspected vulnerabilities. Do not open a public issue before the report has
been assessed.

Include the affected version or commit, platform, required configuration,
reproduction steps, impact, and any relevant logs with credentials and personal
data removed. Use synthetic data and avoid testing against systems or accounts
you do not own or have permission to test.

Kestral grants constrain actions mediated by Kestral. Unsandboxed native app
backends and local tool processes retain the operating-system account's direct
filesystem and network authority. That documented boundary is not itself a
vulnerability, but a bypass of an enforced grant, sandbox, secret, provenance,
or authentication boundary should be reported privately.

General defects and hardening suggestions that do not require confidential
handling can use the public issue templates.
