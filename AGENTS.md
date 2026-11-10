# Repository Guidelines

## Project Structure & Module Organization
Electrocraft is a native Rust circuit simulator and schematic editor. Workspace crates live under `crates/`: `ecs-app` contains the egui desktop shell; `ecs-cli` runs headless netlists; `datasheet-tool` fits device models. Core functionality is split across `ecs-sim`, `ecs-model`, `ecs-expr`, `ecs-lib`, and related crates. `ecs-schematic` owns documents and undo; `ecs-render` draws the canvas; `ecs-catalog` integrates suppliers.

Integration tests live in each crate's `tests/`; catalog fixtures are in `crates/ecs-catalog/tests/fixtures/`. Fonts and icons belong in `assets/`, sample netlists in `examples/`, manuals in `docs/manual/`, and packaging helpers in `scripts/release/` and `packaging/`.

## Build, Test, and Development Commands
Use the pinned toolchain in `rust-toolchain.toml`. Native builds require a C toolchain, CMake 3.25+, clang, and Linux X11/Wayland/OpenGL development headers.

- `cargo run --release -- --sample astable`: launch the desktop app with a sample.
- `cargo run --release -p ecs-cli -- examples/rc.cir`: simulate a netlist and print CSV.
- `cargo build --release --locked -p ecs-app -p ecs-cli -p datasheet-tool`: build distributable binaries.
- `cargo fmt --all --check`: verify formatting.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: lint all targets.
- `cargo test --workspace --locked`: run workspace tests.
- `python3 -m unittest discover -s scripts/release -p 'test_*.py'`: test packaging helpers.

The default workspace member is only `ecs-app`; use `--workspace` or `-p` explicitly.

## Coding Style & Architecture
Follow Rust 2024 conventions: four-space indentation, rustfmt formatting, `snake_case` functions/modules, `PascalCase` types, and `SCREAMING_SNAKE_CASE` constants. Treat decisions in `ELECTROCRAFT-PLAN.SPEC.md` as constraints. Keep `ecs-sim` and lower layers independent of UI crates; use OpenGL/glow, never WGPU. Register user actions in `ecs-app/src/commands.rs`; route document edits through undoable schematic commands.

## Testing Guidelines
Use descriptive `snake_case` test names, Rust unit/integration tests, and existing `proptest` patterns for expression properties. Run focused tests with `cargo test -p ecs-sim --test analytic`. Catalog tests replay fixtures through `MockTransport`; avoid live services. No numeric coverage threshold is configured.

## Commit & Pull Request Guidelines
Follow existing prefixes such as `feat:`, `fix:`, and `fix(release):`. Keep commits focused. PR descriptions should explain behavior changes, validation, and related issues; include screenshots for UI changes and update relevant manuals/specification.

## Security & Configuration
Supplier requests require catalog consent. Store credentials only in the system keychain, never settings, schematic files, or fixtures.
