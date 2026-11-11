The oscilloscope is not a part: its **channels are formulas** over codes, so
measuring never changes the schematic. `scope_add` (`expr`, optional `name`) adds a
channel; `scope_remove` takes the same formula; `scope_clear` removes all. The user
sees lanes on the scope panel; read numbers with `sim_read`.

## Codes

- A net: its label (`out`, `GND`), or `n<wire id>` for unlabelled nets (stable across edits).
- A part terminal: `R1.A.V` (voltage to ground) and `R1.A.I` (current **into** the terminal).
- A two-terminal part: `R1.V` (first pin to second) and `R1.I`.
- Part probes from the definition: `D1.P`, an LED's `D1.brightness`.
- `t`: simulated time.

## Formulas

Any expression over codes: `out`, `in - out`, `R1.I * R1.V` (power), `Q1.C.I / Q1.B.I`
(current gain), `abs(out) * 2`. At most 64 raw signals per scope. A formula with an
unknown code is rejected and the message lists the problem; the app's Codes dialog
lists every code with its meaning.

## Habits

Label the nets you want to watch (`schematic_label` with names like `out`, `in`):
formulas read better than `n17`. Probe voltage across a part with `R1.V` instead of
subtracting two nets. Channel names are display labels only.
