# Livrarr — Engineering Principles

## Purpose

This document states the universal engineering principles that govern how Livrarr is built.

These principles apply regardless of which feature is being built, which crate is being touched, or which contributor is writing the code. They are not Livrarr-specific — they are the properties of any well-built software system, applied here without exception.

For principles specific to Livrarr as a product — what it is, how it thinks about identity, what makes it different — see [`ARCHITECTURE.md`](ARCHITECTURE.md), Part 1.

If the code and these principles diverge, stop and resolve the mismatch. Do not silently accept drift.

A good change makes Livrarr more legible, not merely more capable.

---

## When Principles Conflict

When two principles pull in different directions, resolve in this order:

1. **Data safety** — user data is never silently lost or corrupted
2. **User intent** — what the user asked for happens, completely
3. **Correctness** — one authority, one answer, no contradictions
4. **Simplicity** — the smallest design that satisfies the above

---

## Architectural Priorities

When tradeoffs appear, prefer:

- one strong path over multiple overlapping paths
- coherence over local cleverness
- legibility over brevity
- typed boundaries over stringly glue
- explicit flows over hidden coupling
- the smallest coherent change over the largest ambitious one
- reducing code over growing code when functionality is preserved
- making wrong patterns fail to compile over catching them at runtime
- simple operations over premature distributed complexity

---

## Principles

### 1. Do Not Repeat Yourself (DRY)

Each responsibility should have one clear owner.

### 2. Build for Quality

Build in a manner that encourages, and where possible, mandates quality and minimizes errors.

### 3. Typed Boundaries Are Mandatory

Internal code passes typed values. Serialization happens at the boundary, not in the middle of the codebase.

Status values, state transitions, and enum-like concepts must be Rust enums — not strings, not integers, not opaque JSON passed through the stack. Typed constants where enum modeling is not appropriate. Explicit mappings at every serialization boundary.

### 4. Be Efficient

Use resources only when justified.

### 5. Preserve Reversibility

Reversible processes are inherently less dangerous.

### 6. Keep It Simple

Simple and good enough beats complex and 'perfect.'

---

## Red Flags

Stop and reconsider when a change introduces any of these:

- A second implementation of something that already has one
- Upward crate dependencies (anything depending on `livrarr-server`)
- A user-facing action that doesn't actually change persistent state
- Mutable global state with no obvious owner
- Unbounded channels, caches, or queues
- Raw strings standing in for typed state at internal boundaries
- A new abstraction that exists mostly to feel more "architected"
- A background task with no shutdown path
- A migration that edits an already-shipped file
- A `pub` type that nothing outside its own crate uses — `pub` is a promise no caller consumes; prefer `pub(crate)` (the test and the 2×2 that separates real plumbing from leaks: architecture-review AR-13)

---

## Final Rule

If a senior developer opens this codebase and cannot immediately understand what a file does, who owns a concern, or where a bug would live — that is an architectural failure, not a documentation failure. Fix the structure, not the docs.
