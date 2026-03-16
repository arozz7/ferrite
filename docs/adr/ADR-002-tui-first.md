# ADR-002: TUI First, GUI Later

**Status:** Accepted
**Date:** 2026-03-09

## Context

Recovery tools are often run in constrained environments: a rescue USB, an SSH session over a slow network, or a system with a broken display stack. A graphical UI requires a working desktop environment and is heavier to ship.

## Decision

Build the primary interface as a terminal UI using `ratatui` + `crossterm`. Defer GUI (Tauri or Slint) to Phase 8+.

## Rationale

1. **Rescue environments** — A TUI works over SSH, in a minimal Linux live environment, and on headless servers. A GUI does not.
2. **Backend-first design** — All business logic lives in library crates (`ferrite-imaging`, `ferrite-filesystem`, etc.). The TUI is just one consumer of those libraries. A GUI can layer on top without changing the backend.
3. **Development velocity** — `ratatui` is mature and well-documented. Building the TUI in Phase 7 is straightforward. A GUI framework (Tauri, Slint) adds significant complexity and is better deferred until the backend is proven.
4. **User base** — Data recovery professionals are comfortable with terminal tools (ddrescue, testdisk, PhotoRec are all TUI/CLI).

## Consequences

- Users without terminal comfort will need the GUI phase before adopting Ferrite.
- TUI testing requires either snapshot tests or manual verification.
- The backend library crates must be cleanly separated from UI concerns (no ratatui types leaking into `ferrite-imaging`, etc.).
