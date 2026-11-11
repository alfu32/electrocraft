Schematics are `.ecs` files: pretty-printed JSON with a `format_version` (currently 2)
and stable key order, so they diff cleanly in git. Saving and loading again gives
identical bytes; older versions load through migrations.

## Contents

Metadata; component instances (id, definition id and version, symbol, position, rotation,
mirror, parameter overrides as typed, saved control states, reference designator, an
optional `part_number`, a microcontroller's `code`, an optional `sourcing` block);
wires as polylines; net labels; text; scope channels; component prototypes defined in
the document; simulation settings (speed and time step).

## Tools

- `file_new` creates an empty document in a new tab; `file_open_path` (`path`) opens a
  file in a new tab; `file_open_sample` (`id`) opens a built-in example (led-switch,
  rc-lowpass, astable, counter, rectifier, rc-blocks, pic-blink, divider).
- `file_save` saves to the document's path; an untitled document needs
  `file_save_path` (`path` ending in `.ecs`).
- `file_switch_tab`, `file_next_tab`, `file_close` (asks the user when there are unsaved
  changes), `file_save_all`.
- Documents are tabs; tools act on the active one.

Opening a file does not run anything but the simulation of the circuit; JavaScript
components inside run in a sandbox with no file or network access.
