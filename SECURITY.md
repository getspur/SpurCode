# Security Policy

## Supported versions

SpurCode is a fast-moving project. Security fixes land on `main` and ship in
the next released version. Only the [latest release](https://github.com/getspur/SpurCode/releases)
receives security patches.

## Reporting a vulnerability

**Please do not open a public issue for security vulnerabilities.**

Report privately through GitHub's security advisory form:

<https://github.com/getspur/SpurCode/security/advisories/new>

Include what you can of:

- The affected component (crate, CLI command, or TUI surface)
- Steps to reproduce, including agent CLIs and versions involved
- Impact assessment (data exposure, command injection, credential handling)
- Any proof-of-concept code

We aim to acknowledge reports within a few days and will keep reporters
updated as fixes progress. Coordinated disclosure timelines are worked out
per report. If a report is accepted and fixed, we credit the reporter in the
release notes unless they prefer to remain anonymous.

## Scope notes

SpurCode orchestrates third-party agent CLIs on your local machine:

- Vulnerabilities **in those agent CLIs** should be reported to their own
  projects; SpurCode is not the right venue.
- Misuse of a **user's own API keys** by agents the user configured is
  expected behavior, not a vulnerability — see `docs/user-docs` for the
  permission model. Issues that break the documented permission boundary
  (e.g. a worker escaping worktree isolation or executing outside granted
  permissions) are in scope and welcomed.

## Telemetry

Telemetry tiers, collected fields, and opt-out controls are documented in
[docs/PRIVACY.md](./docs/PRIVACY.md).
