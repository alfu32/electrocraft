Analyses run in the background on a copy of the circuit and open a plot window for
the user; they do not return the numbers to you, except the DC operating point which
annotates the canvas (and `sim_read` then returns it as `dc_op`).

- `analysis_dc_op_annotate`: DC operating point, annotated on the schematic. Fast and
  the best way to get exact bias voltages and currents.
- `analysis_run` with `kind` = `op`, `dc`, `tran` or `ac` runs the analysis with the
  last settings of its dialog, or the defaults (a DC sweep of a source, a transient, an AC
  sweep for Bode plots; the user can adjust them in the dialog). The status says it is running; the plot
  appears for the user when it finishes.
- `analysis_dialog` is not offered: dialogs need the user.

AC analysis needs an AC source: set `AC` on the DC source (`schematic_set_param`
`name=AC value=1`) for the input of the small-signal sweep. Transient runs use the
interactive engine's models with SPICE tolerances (tighter than live mode).

To compare against a SPICE simulator the user can export a netlist (a dialog, so only
the user can: File > Export SPICE Netlist).
