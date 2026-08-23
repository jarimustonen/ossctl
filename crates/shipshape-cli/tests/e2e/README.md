# Release E2E harness

`e2e_release.rs` invokes the compiled `shipshape` binary against a fresh, locally
committed Rust fixture. `git` remains real so the release engine's checkout and
tag behavior is exercised. PATH shims replace `cargo`, `gh`, `curl`,
`sha256sum`, and `shasum`; each shim records argv and returns its test-specific
exit status and stdout. The suite therefore needs neither credentials nor
network access.

Add a scenario by creating a `TempRepo`, configuring `Shims`, and running the
binary through `TempRepo::run`. Assert observable output, journal facts, and
real git state. When asserting a failure, pin `error.code`, never message prose:
messages are operator guidance and may improve without changing the contract.
