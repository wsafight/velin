# Security Policy

## Supported versions

Security fixes are applied to the latest `0.1.x` release and the default branch.
Older development snapshots are not supported.

| Version | Supported |
| --- | --- |
| Latest `0.1.x` | Yes |
| Older versions | No |

## Reporting a vulnerability

Please report suspected vulnerabilities through the repository's private
[GitHub security advisory form](https://github.com/wsafight/velin/security/advisories/new).
Do not open a public issue before a fix is available.

Include the affected version or commit, a minimal reproduction, expected
impact, and any relevant host configuration. Reports involving untrusted
scripts should identify whether the issue crosses a documented resource or
host-effect boundary.

The project will acknowledge the report, validate its scope, and coordinate a
fix and disclosure through the private advisory. Public issues remain suitable
for hardening ideas that do not expose a vulnerability.

## Security boundary

Velin bytecode cannot perform I/O directly. Embedders remain responsible for
allow-listing host commands, validating arguments and replies, enforcing
permissions, and bounding external resources such as time, network, storage,
and output. See [Host protocol](docs/HOST.md) and [Resource limits](docs/LIMITS.md).
