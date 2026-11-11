## Values

Parameters are typed text in **engineering notation**: `4k7`, `100n`, `2.2M`, `1u`,
`0.5`, `1e-3`. Suffixes: `p n u m k M G` (`m` is milli, `M` is mega; `meg` also
works). Units are implied by the parameter (`R` is in ohms) and may be omitted.

`schematic_set_param` takes `refdes`, `name` and `value`. The parameter names of each
component are listed in the `library` topic (resistor `R`, capacitor `C`, inductor
`L`, DC source `V`, sine source `Vamp` `freq` `Voff`, ...). An empty value restores
the default. Invalid text is rejected with a message, nothing changes.

Parameters edited this way update a running simulation in place, without a rebuild:
the cheapest way to sweep a value by hand and watch the result.

## Controls while running

Switches, push buttons, potentiometer wipers and logic toggles are operated with
`sim_control` (`refdes`, `state` = `closed`, `pressed`, `position` or `on`, `value`).
The state is saved with the schematic.

## Part numbers

`schematic_set_part_number` takes a manufacturer part number. If it is in the built-in
database (`part-numbers` topic: 2N2222A, 1N4148, BC547B, 74HC00, PIC16F84A, ...), the
part's model parameters are set to that device's values in the same undo step; an
unknown number is just recorded. A number whose device is a different kind of
component than the part only records the number and says so.

## Sourcing

Manufacturer part number, manufacturer and vendors for buying live in a part's
`sourcing` block (topic `catalogs`), separate from the simulation parameters.
