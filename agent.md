# Agent Instructions

These instructions apply to any AI agent (and any human) working in this repository.

## Always maintain `plan.md`

> **Create a `plan.md` with checkboxes that will be marked `-` for in-progress and `*` for completed to do this.**

This rule is mandatory for every task undertaken in this repository:

- A `plan.md` file MUST exist at the repository root and MUST describe the work as a
  checklist of tasks.
- Every task in `plan.md` is a Markdown checkbox and MUST use exactly one of these markers:
  - `- [ ]` — **not started**
  - `- [-]` — **in progress** (marked with a `-`)
  - `- [*]` — **completed** (marked with a `*`)
- Before starting work on a task, change its marker to `- [-]` (in progress).
- Immediately after finishing a task, change its marker to `- [*]` (completed).
- Keep exactly one logical task in progress at a time where practical.
- When new work is discovered, add new `- [ ]` items to `plan.md` rather than leaving
  them untracked.
- `plan.md` is a living document: keep it in sync with the actual state of the code at all
  times. Never leave it stale.

## Project intent

This repository implements a **WSL1** (not WSL2) Wayland compositor whose final output is
presented on Windows through a **WebGPU** (wgpu) renderer. The compositor runs as a Linux
ELF process inside WSL1; a native Windows process owns the WebGPU device and presents a
single, scalable window. See `plan.md` for the architecture and current status.
