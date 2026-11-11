# Electrocraft quick start

Electrocraft is a native schematic editor with live analog and digital simulation. The current interface is in English. A Microsoft account or supplier account is not needed to use the core editor and simulator.

## Try a circuit

1. Launch Electrocraft and open a built-in example such as **led-switch**, **rc-lowpass**, **astable**, **counter** or **pic-blink** from the Start screen or the File menu's samples.
2. Simulation runs when a circuit opens. Flip the LED example's switch or change a component value in the inspector. Watch wire colours, current dots and LED brightness respond.
3. Use **F5** to pause/resume, **F6** to step and **F7** to reset. A missing ground or floating connection can prevent a circuit from solving.
4. Save the circuit as an `.ecs` document. Open more circuits in document tabs. Undo with **Ctrl+Z**.

## Draw and measure

Place components from the library, connect pins with wires and add ground. **W** starts wire routing; **R** rotates a part. Crossed wires connect only when an endpoint or explicit junction joins them. Use values such as `4k7`, `100n` and `2.2M`.

Choose voltage/current probes from the inspector or add oscilloscope formulas such as `out`, `R1.I` and `R1.V * R1.I`. Label important nets to make formulas readable. Use DC operating-point annotation for steady values, DC sweep to vary a source, transient analysis for time response and AC analysis for frequency response.

## Reuse and extend

Create a component with its own symbol, ports and parameters. Its implementation can be a nested schematic or JavaScript equations. Import SPICE models and subcircuits from File. Supported PIC chips accept assembly source in the program editor. The separate Electrocraft datasheet application fits device-model parameters to measured curves.

## Optional online catalogs

Open **Settings → Online part catalogs**. The tab fills the resizable dialog; scroll for further providers and options. Catalog access is off by default. Enable the master switch, then approve each desired provider. JLCPCB search does not require a key; DigiKey, Mouser, TME and Farnell need your own credentials. Save keys to the Windows credential store, never to a schematic. Search in the Parts panel, place or assign a part, record preferred sourcing and export a CSV bill of materials.

## Optional MCP automation

Start `electrocraft.exe --mcp` from a compatible MCP client's stdio configuration, using the installed executable's actual path. The app exposes tools for creating, wiring, saving, running and reading circuits, plus a built-in manual. Edits appear in its window and can be undone. A separate MCP client may have its own network use, accounts, costs and privacy terms. The app's catalog consent cannot be enabled by these tools.

## Further reference

The `manual/` folder preserves every source topic from `docs/manual/`. Those documents also describe the exact MCP command names. `screenshots.html` is an offline gallery of all supplied screenshots, with captions and import-slot assignments. Support and source: https://github.com/alfu32/electrocraft/issues
