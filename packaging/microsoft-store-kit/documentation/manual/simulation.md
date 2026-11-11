The simulator is a live, interactive engine: modified nodal analysis with Newton
iteration, stepping continuously while it runs. Wire colours show voltage, dots show
current, LEDs glow, and the user sees it all.

## Control

- The simulation **starts running by itself** when a document is opened or created, and restarts after edits. `sim_toggle_run` *toggles* run and pause (its answer says which state it is now in); `sim_step` advances one step while paused;
  `sim_reset` returns to the operating point (use it after a convergence problem).
- The step and speed are part of the document (`simulation` settings in the file).
- `sim_control` operates switches, push buttons, wipers and logic toggles.

## Reading results

`sim_read` returns, when the simulation has produced a snapshot:

- `running`, `time` (simulated seconds), `error` and `warnings`;
- `nets`: voltage of every named net (labels, or the stable `n<wire id>` names);
- `parts`: per designator the branch `currents` and the named `probes` of the
  definition (for example an LED's `brightness`, a resistor's `P`).

Values are only shown while they belong to the current netlist. After a topology edit
(place, wire, delete) the simulation rebuilds first: call `sim_read` again after a
moment. Parameter edits do not rebuild. If `sim_read` reports no data, the simulation
is paused before its first step or the circuit has problems: call `document_describe`
and read `problems`.

## Exact values

The live values move with time, so for a steady answer use
`analysis_dc_op_annotate`: it solves the DC operating point and annotates voltages on
the canvas; `sim_read` then includes them as `dc_op`.

## Typical problems

- **Floating node** or **no ground**: add a ground part.
- **Singular or non-convergent circuit**: ideal sources in loops, shorted voltage
  sources, or a node with only current sources. Add a small series resistance.
- **Diagonal-looking wires after moving parts**: harmless but untidy; delete and redraw.
