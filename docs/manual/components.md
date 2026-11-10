Any component, built in or custom, has a symbol with named ports and an implementation.
A schematic can carry its own component prototypes (`local.*` ids), implemented by a
nested schematic or by JavaScript. They appear in the library under "This schematic".

The tools cannot open the component editor (it is a dialog-style window for the user:
symbol drawing, ports, parameters, implementation). What you can do:

- Use existing components: `schematic_place` with a `local.` id once the user made one.
- Describe what you want and let the user create it (`component_new`, `component_edit`
  are not exposed as tools).
- Write a user component as a JavaScript file in the user components folder (File >
  User Components Folder): it is hot-reloaded when saved. The `scripting-api` topic has
  the TypeScript declarations and three reference components; equations are written as
  plain JavaScript functions over voltages, or as `e` templates.

## Hierarchy

`component_enter` (`refdes`) opens a part's inner schematic on the canvas; later
edits (`schematic_place`, ...) then act on the inner schematic until `component_exit`
returns to the enclosing one. Port labels are created for every pin. The inner
schematic simulates on its own while open.

## Rules the engine enforces

Components never write solver stamps: they declare branches as expressions of pin
voltages (and charges for dynamics) and the engine derives the stamps, derivatives and
integration. Behavior hooks (digital events) run on events, never inside the Newton loop.
