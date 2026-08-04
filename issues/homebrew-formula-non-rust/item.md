---
created: 2026-08-04
updated: 2026-08-04
type: task
status: open
priority: normal
epic: ossctl-phase4-build
---

# Generate non-Rust Homebrew formulas (drop the hardcoded cargo stanza)

## Description

render_formula hardcodes depends_on rust => :build + cargo install, assuming every homebrew target is a Rust project. The adapter only sees Ecosystem::Binary; thread the real project language/build recipe from the contract/facts so Go/Node/etc. formulas build. Today both consumers (ossctl, issuectl) are Rust CLIs, so this is deferred. Raised by /llm-review of homebrew-adapter-first-formula (F10).
