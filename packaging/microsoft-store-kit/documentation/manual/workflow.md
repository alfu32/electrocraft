Build in small verified steps. The loop is: **place, wire, describe, run, read**.

## Recipe

1. `file_new` for an empty schematic (or `file_open_path`, `file_open_sample`).
2. Plan coordinates on the 20-unit grid before placing (topic `schematic`): sources
   and loads on a rectangle, ground below. Pins sit 40 units from a two-terminal
   part's position (for a resistor at (80,-40) the pins are (40,-40) and (120,-40)).
3. `schematic_place` each part with `def` (ids are in the `library` topic), `x`, `y`,
   and `rot` (0..3 quarter turns clockwise). The answer names the new designator
   and describes its pins; read pin positions from it instead of computing them.
4. `schematic_set_param` for values (`R` for resistors, `V` for a DC source, `C`,
   `L`...; topic `parameters`).
5. `schematic_wire` with `points`, the polyline from one pin to another. Segments that
   are not horizontal or vertical get an elbow automatically. A wire connects only at
   its ends (to a pin, to another wire's end, or onto another wire's interior, which
   makes a junction). Crossing wires do not connect.
6. **Always** place a `core.ground` part (or a net label `GND`) in every circuit.
7. `document_describe`: check `problems` and `unconnected` (pins touching nothing). Fix
   before running.
8. The live simulation **runs by default** whenever a document is open (and restarts by itself after edits), so just wait a moment and call `sim_read` (it reports `running`). `sim_toggle_run` *toggles* between running and paused: call it only to pause or to resume. For
   exact values use `analysis_dc_op_annotate` (annotates the canvas) then `sim_read`.
9. Add scope channels (`scope_add`, topic `scope`) to watch waveforms; the user sees them.
10. `file_save_path` with a `.ecs` path when the user wants the file kept.

## Worked example: a 5 V source, 330 ohm resistor and LED

```
schematic_place   def=core.vdc       x=0   y=0        # V1: P at (0,-40), N at (0,40)
schematic_place   def=core.resistor  x=80  y=-40      # R1: A (40,-40), B (120,-40)
schematic_place   def=core.led       x=160 y=0 rot=1  # D1: A (160,-40), K (160,40)
schematic_place   def=core.ground    x=0   y=80       # G at (0,80)
schematic_set_param refdes=R1 name=R value=330
schematic_wire    points=[[0,-40],[40,-40]]           # V1.P to R1.A
schematic_wire    points=[[120,-40],[160,-40]]        # R1.B to D1.A
schematic_wire    points=[[0,40],[0,80]]              # V1.N to ground
schematic_wire    points=[[160,40],[160,80],[0,80]]   # D1.K to ground
document_describe                                     # no problems, nothing unconnected
sim_read                                              # LED lit: D1 current about 9.4 mA, node at 1.9 V
```

## Habits that avoid trouble

- Describe after a batch of edits, not after every call.
- Prefer parameter edits (`schematic_set_param`) to deleting and re-placing: they keep
  wiring and refdes, and update the running simulation in place.
- If a wire attaches nowhere, the pin positions you assumed were wrong: describe and
  use the reported pin coordinates.
- Rotation changes pin positions: place with the final `rot`, or use
  `schematic_orient_part` (attached wires stretch, which may leave diagonal runs).
