# Certification notes

## Capability justification — runFullTrust

Electrocraft is an existing native Windows desktop application packaged as MSIX. It uses a native OpenGL window and a local circuit-simulation engine, opens and saves user-selected schematic files, imports user-selected models and JavaScript components, exports netlists and CSV bills of materials, reads/writes local application settings and caches, and stores optional supplier credentials in Windows Credential Manager. Its optional MCP mode communicates over standard input/output with a client launched by the user. It does not require administrator rights for normal editing or simulation.

## Reviewer workflow

1. Install the x64 package on x64 Windows or the ARM64 package on ARM64 Windows. Launch Electrocraft from Start.
2. Open **led-switch** from the built-in samples. Operate its switch and observe LED brightness/current animation. No account, key, sign-in or network connection is needed.
3. Open **rc-lowpass** and inspect the input/output scope. Pause/resume with F5. Open **astable** for the transistor multivibrator.
4. Place a resistor, wire components and ground, change a value and undo. Save and reopen an `.ecs` document.
5. Open **pic-blink** and inspect its program editor. Open Settings and select **Online part catalogs** to see provider consent and credential fields. Catalogs should remain off unless explicitly enabled by the reviewer.
6. Supplier APIs require the reviewer's own provider credentials except JLCPCB. This does not block local editing, simulation, samples or file operations. Catalog availability depends on independent third parties.

The optional datasheet tool is a companion executable. The optional MCP integration requires a separately configured compatible client; core app testing does not require one.

## Prepared artifacts and remaining platform checks

Release 0.1.13 passed repository formatting, Clippy, Rust tests, packaging tests and all six release platform jobs. Release workflow: https://github.com/alfu32/electrocraft/actions/runs/42880210045

The Windows release workflow builds/packages/signs the MSIX and checks installation. This package verifies the downloaded MSIX identity, architecture, resource language and version against the reserved identity and recorded checksums. These checks are not Microsoft certification and do not replace Windows App Certification Kit or visual testing on a Windows desktop. No WACK result is claimed.

## Notes for publication

The app is an electronics simulation/design tool, not a PCB-layout tool or a guarantee of physical circuit behavior. PIC execution uses the app's supported simulation models; do not promise complete emulation of every peripheral or arbitrary firmware. Results depend on model and circuit assumptions. All interface screenshots supplied here are English. Store-listing translations do not add application translations.
