# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Electrocraft: a Rust circuit simulator with a live schematic editor. The
specification, decisions (D1–D14) and per-phase status are in
`ELECTROCRAFT-PLAN.SPEC.md`; treat the decision log as constraints.

## Commands

```sh
cargo run -- --sample astable                       # desktop app (crate ecs-app, binary electrocraft)
cargo run -- --command-catalog                      # every command id (the future MCP tools)
cargo run -- --sample rc-lowpass --commands cmds.json   # replay [{"id": ..., "args": {...}}]
cargo run -p ecs-cli -- examples/rc.cir             # headless runner (binary electrocraft-cli)
cargo run -p datasheet-tool -- --example npn        # model fitting app (binary electrocraft-datasheet)
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p ecs-sim --test analytic               # one integration test file
cargo test -p ecs-schematic net_names               # tests whose name contains a filter
python3 -m unittest discover -s scripts/release -p 'test_*.py'
python3 scripts/capture_screenshots.py              # needs a release build, Xvfb, xdotool, ImageMagick
python3 scripts/release/appimage.py                 # AppGlider-friendly AppImage into dist/
```

`default-members` is only `crates/ecs-app`: pass `--workspace` or `-p` for the rest.
Toolchain is pinned in `rust-toolchain.toml` (edition 2024). ngspice is optional
locally (cross-check tests skip without it; CI installs it).

## Architecture

- `ecs-units` engineering notation · `ecs-expr` parser, hash-consed graph, autodiff tapes, `Display` that round-trips through `parse` · `ecs-model` `ComponentDef` contract, logic levels and families, `Behavior`/`EventCtx`, subcircuits (`SubElement` params may be expressions over the subcircuit's params) · `ecs-lib` built-ins as expressions (`digital.rs` for the event-driven parts) · `ecs-sim` MNA engine (`circuit.rs`: compiler, flattening, Newton, digital event queue), analyses (`analysis.rs`) and live thread (`live.rs`) · `ecs-script` QuickJS host (`prelude.js` is the JS API, `electrocraft.d.ts` its types) · `ecs-spice` import/export · `ecs-schematic` document, symbols (`symbols::register` for generated ones), connectivity, commands, undo · `ecs-render` Raylib thread drawing display lists · `ecs-app` egui shell · `datasheet-tool` model fitting app (`fit.rs` is headless).
- Data flow in the app: `Schematic` (document) → `catalog::build` → `Built` (netlist, net names, pin→net maps) → `ecs_sim::live::Command::Rebuild`. Edits that only change parameters are sent as `SetParam` instead of a rebuild. Snapshots come back through a triple buffer and scope samples through an SPSC ring tagged with the probe-list generation.
- Every user action is a `Cmd` with a registry id in `crates/ecs-app/src/commands.rs` (D14, MCP-ready). Add new actions there, not only to a menu; the catalog test checks every id builds. Every document change goes through an `ecs_schematic::Command` so it is undoable.
- The oscilloscope is not a component: `Schematic.scope` holds channels whose `expr` is a formula over codes (`out`, `R1.I`, `Q1.B.V`, `U1.OUT.I`, part probes like `D1.P`, and `t`). `ecs-app/src/scope.rs` builds the code table from `Built` and compiles formulas to tapes over at most `MAX_CHANNELS` raw signals. Unlabelled nets are named stably (`n<wire id>` / `p<part>_<pin>`) so codes survive edits.
- Document-local components (`Schematic.components`, `LocalComponent`) carry their own symbol and an implementation (inline or external schematic, inline or external JavaScript); they compile into the catalog under `local.` ids.
- Interactive mode integrates with θ = 0.6 (damped trapezoidal) on purpose: plain trapezoidal rings on stiff edges. Analyses use θ = 0.5.
- `ecs-sim` and below must never depend on UI crates. No WGPU anywhere (D4): eframe uses `glow`.
- Components never write stamps: add parts by declaring branches as expressions in `ecs-lib`, a symbol in `ecs-schematic::symbols` (pins on the 20-unit grid, names matching the definition's pins) and, if it belongs in the ribbon, an entry in `chrome.rs`.
- UI changes: keep SolidCraft's palette tokens in `theme.rs` (D10) and Wirecraft's canvas conventions (D11); refresh screenshots and the spec's section 9.

## Release

The app version is the literal `version` in `crates/ecs-app/Cargo.toml`. Tags equal it with no `v` prefix. AppImage names, desktop keys and icon placement are what AppGlider reads (spec section 13); `scripts/release/test_release.py` guards them.

At the end of every batch of work: run the checks above, commit, tag the version (bump it first if the tag exists), push the branch and the tag, then trigger the release on the tag with `gh workflow run release.yaml --ref X.Y.Z` (the workflow is manual-dispatch only and refuses non-tag refs or a tag that differs from the version).
