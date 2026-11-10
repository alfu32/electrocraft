PIC microcontrollers run real machine code in the simulation: PIC12F675, PIC16F84A,
PIC16F628A, PIC16F877A (MPASM assembly), PIC24FJ64GA002 (XC16) and PIC32MX250F128B
(MIPS32, GNU as syntax). The part's program is its `code` property; the chips run as
behaviors on the event layer, with 5 V logic pins that drive and read the analog nets.

## Setting a program

`schematic_set_program` (`refdes`, `code`) replaces the assembly source in one undo
step; the app assembles it (errors name the line) and restarts the chip. An empty
program leaves every pin an input. `mcu_edit_program` is not exposed (it opens the
editor window); the user can open it from the inspector.

Syntax notes:

- PIC16 / PIC12 (MPASM): default radix hex, so write decimals as `D'10'` or `.10`;
  starts at `ORG 0`, interrupts at 4. Use `BANKSEL` or the explicit bank bits.
- PIC24 (XC16): `#literal`, working registers `W0..W15`, `[Wn++]` addressing; starts at `__reset`.
- PIC32 (MIPS): `$registers`, branch delay slots are filled with NOPs (`.set reorder`),
  starts at `main`.

Each template in the program editor blinks an LED on a pin with a 250 ms period; the
sample `pic-blink` (`file_open_sample`) shows one wired to an LED.

## Wiring

Power the chip (VDD, VSS pins) and ground it, connect the pins you drive, and the
program does the rest. `document_describe` lists the chip's pin names; I/O ports are
named `RA0`, `RB3` and so on.
