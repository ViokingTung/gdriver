# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Reporting a Vulnerability

**Do not open a public issue.** 

Report security vulnerabilities privately via GitHub Security Advisories:

1. Go to the [Security tab](https://github.com/gdriver/gdriver/security)
2. Click **Report a vulnerability**
3. Describe the issue in detail

You can also email `security@example.com`. Expect an initial response within 72 hours.

## What to Include

- Detailed description of the vulnerability
- Steps to reproduce
- Affected versions / components
- Any potential mitigations you've identified

## Disclosure Policy

We follow coordinated disclosure:

1. Reporter submits vulnerability privately
2. We acknowledge within 72 hours and begin investigation
3. We develop and test a fix
4. We release the fix and publish a security advisory
5. Credit given to the reporter (unless they prefer anonymity)

## OAuth Credential Safety

gDriver uses OAuth 2.0 to access Google Drive. A few important design decisions:

- **Client secrets** are never embedded in the binary — they are loaded from environment variables or OS keyring at runtime
- **Access tokens** are stored exclusively in the platform keyring (never in the database or on disk)
- **Refresh tokens** are stored exclusively in the platform keyring
- All Google API communication is **HTTPS-only** — no plaintext HTTP fallback

If you discover a flaw in credential handling, report it immediately through the private channel above.
