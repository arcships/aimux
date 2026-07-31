# Security Policy

## Supported Versions

aimux is pre-1.0 software. Security fixes are applied to the latest `master`
branch only.

| Version | Supported          |
|---------|--------------------|
| 0.1.x   | :white_check_mark: |
| < 0.1   | :x:                |

## Reporting a Vulnerability

**Please do not open public GitHub issues for security vulnerabilities.**

Instead, report vulnerabilities privately:

1. Open a **private security advisory** via GitHub:
   `https://github.com/arcships/aimux/security/advisories/new`, or
2. Email the maintainers at `security@arcships.com` (if available).

Please include:
- A description of the issue and its potential impact.
- Steps to reproduce, including a minimal proof-of-concept if possible.
- The affected version / commit.

We will acknowledge receipt within **72 hours** and aim to provide an initial
assessment within **7 days**. Coordinated disclosure is preferred; please do
not publish details until a fix is released.

## Scope

aimux is a provider access layer that makes outbound HTTP calls to LLM
providers using user-supplied API keys. Security-relevant issues include:

- Mishandling of API keys or other secrets.
- Request smuggling, header injection, or unsafe URL handling.
- Deserialization or parsing bugs that could cause panics or memory-safety
  issues in the Rust core or FFI boundary.
- Flaws in the FFI handle lifecycle that could lead to use-after-free or
  leaks across language bindings.

Out of scope:
- Vulnerabilities in upstream LLM provider APIs themselves.
- Issues that require already-compromised API keys.
- Denial of service via intentionally malformed provider responses that only
  affect a single request (report as a bug instead).

## Hardening Notes

- The Rust core is built with `panic = "abort"` in release; panics abort the
  process rather than unwind. Bindings should treat any panic as fatal.
- API keys are read from the environment or explicit config; aimux never logs
  key material. Do not place keys in code or committed config.
