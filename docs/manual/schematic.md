## Coordinates

World coordinates are in units on a **20-unit grid**: x grows right, y grows **down**.
Positions you give are snapped to the grid. Parts are placed by their position (their
origin); the pins are at fixed offsets from it, rotated with the part.

Rotation `rot` is quarter turns **clockwise** on screen (0..3); `mirror` flips
horizontally before rotating.

## Pin offsets at rot 0 (from the part's position)

| Part | Pins |
| --- | --- |
| resistor, capacitor, inductor, diode, LED, switch | A (-40,0), B (40,0) (diode and LED: A anode, K cathode) |
| DC source `core.vdc` | P (0,-40), N (0,40) |
| ground | G (0,0): the symbol hangs below the pin |
| NPN `core.npn` | C (20,-40), B (-40,0), E (20,40) |
| op-amp | INP (-60,20), INN (-60,-20), OUT (60,0) |
| potentiometer | A (-40,0), W (0,-40), B (40,0) |
| NAND-type gates | A (-40,-20), B (-40,20), Y (40,0) |
| 555 timer | TRIG (-60,-20), THR (-60,0), RST (-60,20), DIS (-60,40), VCC (60,-20), OUT (60,0), GND (60,20) |

These are examples. `document_describe` reports every part's pins in **world**
coordinates (already rotated), which is what you wire to. Trust it over this table.

## Wires

`schematic_wire` takes `points`, a polyline `[[x,y],[x,y],...]`. A pair of points that
is not horizontal or vertical gets an elbow (horizontal first). Rules:

- A wire connects at its **ends**: to a pin it ends on, to another wire's end, or to
  the interior of another wire (a **junction**, drawn automatically).
- A wire that merely passes over a pin or another wire does not connect.
- Crossing wires do not connect.
- Redundant points are removed. A wire needs two distinct points.
- `schematic_delete_wire` takes the wire's id (`wire_id`) from `document_describe`.

## Net labels and text

`schematic_label` (name, x, y) names a net: labels with the same name are connected
even without a wire between them, which keeps drawings tidy. `GND` and `0` are
ground. Place the label so its point lies on a wire or pin. `schematic_text` adds a
free annotation.

## Reference designators

Parts get `R1`, `C1`, `Q1`... automatically from the definition's prefix. They are
how every tool names a part. Renaming is done in the inspector, not by a tool.

## Moving, rotating, deleting, selecting

`schematic_move_part` moves to an absolute position (attached wires stretch and stay
orthogonal). `schematic_orient_part` sets the absolute rotation and mirroring.
`schematic_delete_part` removes a part and its attached wire ends. `schematic_select`
selects parts by designator so `edit_copy`, `edit_duplicate`, `edit_rotate`,
`edit_mirror` and `edit_delete` apply to them. `edit_undo` and `edit_redo` step
through the user's undo history: each tool call that changes the schematic is one step.

## Clipboard

Copy and Cut (`edit_copy`, `edit_cut`) put the selection on the system clipboard as text (JSON), so it can be pasted into another Electrocraft window, a file or a chat. In the window, Ctrl+V and the Paste menu read the system clipboard: a copied selection pastes at the pointer, a whole `.ecs` file's text pastes its contents, and short plain text (up to 200 characters) becomes a note. Paste needs the pointer, so it is not an MCP tool.
