# Security Policy

## Supported Versions

tokcost is pre-1.0. Security fixes land on the latest release; there are no
maintained backport branches.

| Version | Supported |
| ------- | --------- |
| 0.1.x   | ✅        |

## Reporting a Vulnerability

**Please do not open a public issue for a security vulnerability.**

Report it privately through GitHub's
[private vulnerability reporting](https://github.com/abhiprd2000/tokcost/security/advisories/new)
for this repository. If that isn't available to you, contact the maintainer
directly through their GitHub profile.

Please include:

- What the issue is and where in the code it lives
- Steps to reproduce, ideally with a concrete input
- What an attacker could achieve with it

You can expect an initial response within about a week. If the report is
confirmed, I'll work on a fix and credit you in the release notes unless you'd
rather stay anonymous.

## Threat Model

Knowing what tokcost does and doesn't do makes it easier to judge whether
something is a real issue:

**tokcost makes zero network calls.** Not for pricing, not for telemetry, not
for vocabulary downloads — everything it needs is compiled into the binary. Any
observed outbound connection from tokcost itself is a genuine bug and worth
reporting.

**tokcost has zero runtime dependencies.** The `[dependencies]` section of
`Cargo.toml` is empty, so the supply chain is this crate plus the Rust standard
library. There is no transitive dependency tree to audit.

**tokcost reads, but never writes, the files you point it at.** The counting
path opens inputs read-only. It does not create, modify, or delete files.

**No `unsafe` code.** The crate is entirely safe Rust.

### Where untrusted input flows

These are the interesting surfaces for review:

- **File and stdin content** is treated purely as bytes to tokenize. Invalid
  UTF-8 is replaced lossily rather than rejected, and content is never
  evaluated, executed, or interpreted as anything but text.
- **`TK_PRICES`** is parsed from the environment. Malformed entries are skipped
  rather than failing the run; values are parsed as plain floats.
- **`tokcost meter -- <command>`** spawns a child process. The command is taken
  from your argv and executed **without a shell** (`std::process::Command`
  passes arguments directly), so shell metacharacters in arguments are not
  interpreted. tokcost applies no sandboxing to the wrapped command — it runs
  with your full privileges, exactly as if you had run it yourself.
- **Terminal output** may include ANSI escape sequences for color, but only
  when stdout is a TTY. Content read from input files is never echoed back to
  the terminal, so a file containing escape sequences cannot use tokcost to
  manipulate your terminal.

### Out of scope

- The accuracy of the pricing table (it's a dated snapshot — see the README)
- Token counts that differ from a provider's own counter (report those as
  regular bugs; they're valuable, just not security issues)
- The behavior of a command you explicitly wrap with `tokcost meter`
