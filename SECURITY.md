# Security Policy

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues, discussions, or
pull requests.**

Report privately using **GitHub's [Private Vulnerability Reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)**:
open the repository's **Security** tab → **Report a vulnerability**.

Include, as far as you can: the affected version or commit, the component and threat surface
(for example the subprocess a command invokes, or a prebuilt release binary), reproduction
steps or a proof-of-concept, and the impact you observed.

`shipshape` is a local command-line tool: it does not run a network service, but it **invokes
subprocesses** (`git`, `cargo`, and release adapters) and is intended to **distribute prebuilt
binaries** via GitHub Releases and a Homebrew tap. Those — subprocess execution today, and the
release binaries once published — are its primary threat surfaces.

<!-- shipshape-security:supported-versions-start -->
## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅        |
<!-- shipshape-security:supported-versions-end -->

## What to Expect

- We will acknowledge your report as soon as we can.
- We will confirm the issue and determine its severity, and keep you informed of progress.
- We ask that you give us a reasonable window to release a fix before any public disclosure —
  we practice coordinated disclosure and will credit you (unless you prefer to remain anonymous).

## Safe Harbor

We consider good-faith security research conducted under this policy to be authorized. We will
not pursue or support legal action against researchers who act in good faith, avoid privacy
violations and service disruption, and give us a reasonable time to respond before disclosure.

This safe harbor covers only assets this project controls — its source code, this repository,
and the binaries it publishes. It does not authorize testing of GitHub, crates.io, Homebrew, or
other third-party services, whose own policies continue to apply.
