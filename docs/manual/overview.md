Electrocraft is a circuit simulator with a live schematic editor. You are connected
to the running application: every tool acts on the window the user is looking at, so
they see parts appear, wires connect and the simulation run. Be deliberate: the user
can undo with Ctrl+Z, but a wrong turn costs their attention.

## How the tools map to the app

- Every user action in the app is a **command** with a stable id (`schematic.place`,
  `sim.toggle_run`, ...). Each becomes one tool; the id's dots become underscores
  (`schematic_place`, `sim_toggle_run`). The `commands` topic lists all of them
  with their arguments.
- **Read-only tools** report state: `document_describe` (parts, pins with world
  positions and nets, wires, labels, problems), `sim_read` (live voltages and
  currents) and `parts_results` (results of the last catalog search).
- `get_manual` is this manual. Resources `manual://index` and `manual://<topic>`
  carry the same text.

## What a tool call returns

Editing tools answer with the status line the app shows ("Placed R1"), and a short
change summary (`parts 2 -> 3, wires 4 -> 4`). A call that changed nothing says so;
that usually means a wrong designator, a missing argument or points that make no
wire. Errors come back as tool errors with the reason.

## What you cannot do

- Open file dialogs or the editors (they need the user): use the `_path` variants
  (`file_open_path`, `file_save_path`, `bom_export_path`).
- Turn online part catalogs on. Only the user can, in Settings. Until then
  `parts_search` explains that catalogs are off.
- Operate the pointer: there is no drag or click, use `schematic_move_part`,
  `schematic_wire` and `schematic_select`.

## Orientation

Units are SI with engineering suffixes in text values (`4k7`, `100n`, `2.2M`).
Positions are world coordinates on a 20-unit grid (topic `schematic`). Start with
the `workflow` topic.
