#!/usr/bin/env python3
"""Drive the real Electrocraft window in a private Xvfb display and capture the
screenshots used by the specification (docs/screenshots/).

Requires Xvfb, xdotool and ImageMagick's `import`. Standard library only.

    python3 scripts/capture_screenshots.py --binary target/release/electrocraft
"""

import argparse
import json
import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "docs" / "screenshots"
W, H = 1600, 1000


class Session:
    def __init__(self, binary, display, home, args, commands=None):
        if commands is not None:
            path = Path(home) / "commands.json"
            path.write_text(json.dumps(commands))
            args = (*args, "--commands", str(path))
        self.env = dict(os.environ, DISPLAY=f":{display}", HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"))
        self.env.pop("WAYLAND_DISPLAY", None)
        self.proc = subprocess.Popen([binary, "--size", f"{W}x{H}", *args], env=self.env,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(5)
        # No window manager runs in Xvfb: give the window keyboard focus.
        subprocess.run(["xdotool", "search", "--sync", "--name", "Electrocraft", "windowfocus"],
                       env=self.env, check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(0.3)

    def x(self, *cmd):
        subprocess.run(["xdotool", *map(str, cmd)], env=self.env, check=True)
        time.sleep(0.25)

    def shot(self, name, wait=0.8):
        time.sleep(wait)
        subprocess.run(["import", "-window", "root", str(OUT / name)], env=self.env, check=True)
        print("captured", name)

    def close(self):
        self.proc.terminate()
        self.proc.wait(timeout=10)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--binary", default=str(ROOT / "target" / "release" / "electrocraft"))
    ap.add_argument("--display", type=int, default=91)
    args = ap.parse_args()
    for tool in ("Xvfb", "xdotool", "import"):
        if not shutil.which(tool):
            raise SystemExit(f"{tool} is required")
    OUT.mkdir(parents=True, exist_ok=True)
    xvfb = subprocess.Popen(["Xvfb", f":{args.display}", "-screen", "0", f"{W}x{H}x24", "-noreset"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1.5)
    home = Path(tempfile.mkdtemp(prefix="electrocraft-shots-"))
    run = lambda *a, commands=None: Session(args.binary, args.display, home, a, commands)
    try:
        s = run()
        s.shot("01-start-screen.png")
        s.close()

        s = run("--sample", "astable")
        s.shot("02-workspace-astable-live.png", wait=2.5)
        s.x("mousemove", 237, 56, "click", 1)
        s.x("mousemove", 1300, 860)
        s.shot("03-ribbon-simulate-tab.png")
        s.x("mousemove", 136, 56, "click", 1)
        # Right-to-left crossing window: dashed green, touches what it crosses.
        s.x("mousemove", 1010, 520, "mousedown", 1)
        for x, y in ((950, 480), (880, 420), (840, 400), (800, 380)):
            s.x("mousemove", x, y)
        s.shot("04-crossing-window-selection.png", wait=0.5)
        s.x("mouseup", 1)
        s.shot("05-selection-inspector.png")
        s.close()

        s = run("--sample", "led-switch")
        s.shot("06-led-switch-open.png", wait=1.5)
        s.x("mousemove", 675, 352, "click", 1)
        s.x("mousemove", 1350, 700)
        s.shot("07-led-switch-closed-current.png", wait=1.5)
        # Placement shadow follows the pointer, snapped to the grid.
        s.x("mousemove", 364, 105, "click", 1)
        s.x("mousemove", 1250, 470)
        s.x("mousemove", 1262, 476)
        s.shot("08-placement-shadow.png")
        s.x("key", "Escape")
        # Wire tool: orthogonal preview from a pin.
        s.x("key", "w")
        s.x("mousemove", 1072, 640, "click", 1)
        s.x("mousemove", 1300, 560)
        s.x("mousemove", 1310, 552)
        s.shot("09-wire-routing.png")
        s.x("key", "Escape")
        s.x("key", "Escape")
        s.x("mousemove", 1300, 620, "click", 3)
        s.shot("10-context-menu.png")
        s.x("key", "Escape")
        s.x("key", "ctrl+k")
        s.x("type", "--delay", "60", "place led")
        s.shot("11-command-palette.png")
        s.close()

        s = run("--sample", "rc-lowpass")
        s.shot("12-scope-rc-lowpass.png", wait=4.0)
        s.close()

        s = run("--sample", "divider")
        s.x("mousemove", 805, 480, "click", 1)
        s.x("mousemove", 1100, 760)
        s.shot("13-inspector-live-slider.png", wait=1.0)
        s.close()

        s = run("--sample", "counter")
        s.x("mousemove", 230, 56, "click", 1)
        s.x("mousemove", 1400, 700)
        s.shot("15-digital-ribbon-counter.png", wait=3.0)
        s.close()

        s = run("--sample", "astable")
        s.x("mousemove", 600, 905, "click", 1)
        s.x("mousemove", 930, 905, "click", 3)
        s.x("mousemove", 1400, 700)
        s.shot("16-oscilloscope-cursors.png", wait=1.0)
        s.close()

        # Scope section of the inspector: a checkbox per terminal quantity.
        s = run("--sample", "astable")
        s.x("mousemove", 927, 352, "click", 1)
        s.x("mousemove", 1450, 450, "click", 5)
        s.x("click", 5)
        s.x("mousemove", 1100, 560)
        s.shot("21-scope-inspector.png", wait=1.5)
        s.close()

        s = run("--sample", "astable")
        s.x("mousemove", 775, 642, "click", 1)
        s.x("mousemove", 300, 900)
        s.shot("22-scope-floating.png", wait=1.5)
        s.close()

        # Component editor on a schematic-defined block: Symbol tab with a new pin.
        s = run("--sample", "rc-blocks")
        s.shot("25-custom-component-sample.png", wait=3.0)
        s.x("mousemove", 678, 306, "click", 1)
        s.x("key", "ctrl+e")
        s.x("mousemove", 381, 80, "click", 1)
        s.x("mousemove", 661, 124, "click", 1)
        s.x("mousemove", 540, 460, "click", 1)
        s.x("mousemove", 925, 554, "click", 1)
        s.x("mousemove", 600, 400)
        s.shot("23-component-editor-symbol.png", wait=0.8)
        s.close()

        s = run("--sample", "rc-blocks")
        s.x("mousemove", 678, 306, "click", 1)
        s.x("key", "ctrl+Return")
        s.x("mousemove", 1300, 560)
        s.shot("24-component-inner-schematic.png", wait=2.0)
        s.close()

        # Code editor: function-style template, highlighting and a suggestion list.
        s = run("--sample", "rc-lowpass", commands=[{"id": "component.new", "args": {}}])
        s.x("mousemove", 480, 99, "click", 1)
        s.x("mousemove", 400, 350, "click", 1)
        s.x("key", "End")
        s.x("key", "Return")
        s.x("type", "--delay", "80", "const k = Math.ta")
        s.shot("26-component-code-editor.png", wait=0.6)
        s.close()

        s = run("--sample", "rc-lowpass")
        s.x("mousemove", 90, 730, "click", 1)
        s.x("type", "--delay", "80", "C1.")
        s.shot("27-scope-formula-suggestions.png", wait=0.6)
        s.close()

        # Documents in tabs; + opens another one in a new tab.
        s = run("--sample", "astable", commands=[{"id": "file.open_sample", "args": {"id": "rc-lowpass"}},
                                                  {"id": "file.open_sample", "args": {"id": "counter"}}])
        s.x("mousemove", 770, 17, "click", 1)
        s.shot("28-document-tabs.png", wait=2.0)
        s.close()

        s = run("--sample", "astable", commands=[{"id": "app.settings", "args": {}}])
        s.shot("29-settings.png", wait=2.0)
        s.close()

        # Full-height catalog settings with provider options, without any search.
        cfg = home / ".config" / "electrocraft"
        cfg.mkdir(parents=True, exist_ok=True)
        (cfg / "settings.json").write_text(json.dumps({"catalogs": {
            "master": True, "enabled": ["jlcpcb", "digikey", "mouser", "tme", "farnell"]}}))
        s = run("--sample", "astable", commands=[{"id": "app.settings", "args": {}}])
        s.x("mousemove", 675, 244, "click", 1)
        s.x("mousemove", 1300, 860)
        s.shot("36-settings-catalogs.png", wait=1.0)
        s.close()
        (cfg / "settings.json").unlink()

        # Microcontroller: the PIC sample and its program editor.
        s = run("--sample", "pic-blink")
        s.x("mousemove", 1500, 900)
        s.shot("30-pic-blinker.png", wait=3.0)
        s.close()
        s = run("--sample", "pic-blink", commands=[{"id": "mcu.edit_program", "args": {"refdes": "U1"}}])
        s.x("mousemove", 1500, 900)
        s.shot("31-pic-program.png", wait=2.0)
        s.close()

        # Part numbers: Q2 as a 2N2222A.
        s = run("--sample", "astable", commands=[{"id": "schematic.set_part_number", "args": {"refdes": "Q2", "number": "2N2222A"}}])
        s.x("mousemove", 916, 402, "click", 1)
        s.x("mousemove", 1100, 560)
        s.shot("32-part-number.png", wait=1.5)
        s.close()

        # Logic chips in the components panel.
        s = run("--sample", "counter")
        s.x("mousemove", 120, 218, "click", 1)
        s.x("type", "--delay", "60", "74x")
        s.x("mousemove", 1100, 560)
        s.shot("33-logic-chips.png", wait=1.0)
        s.close()

        s = run("--sample", "rc-lowpass", commands=[{"id": "analysis.run", "args": {"kind": "ac"}}])
        s.shot("17-analysis-ac-bode.png", wait=3.0)
        s.close()

        s = run("--sample", "astable", commands=[{"id": "analysis.run", "args": {"kind": "tran"}}])
        s.shot("18-analysis-transient.png", wait=8.0)
        s.close()

        s = run("--sample", "astable", commands=[{"id": "file.import_spice", "args": {}}])
        s.x("mousemove", 620, 170, "click", 1)
        for line in [".model D1N4148 D(IS=2.52n RS=.568 N=1.752 BV=100 IBV=100u CJO=4p TT=20n)",
                     ".model Q2N3904 NPN(IS=6.734f BF=416.4 NF=1 VAF=74.03 CJE=4.493p IKF=66.78m)",
                     ".subckt OPA1 inp inn out", "Rin inp inn 10meg", "E1 n1 0 inp inn 1e5",
                     "R1 n1 n2 1k", "C1 n2 0 15.9u", "E2 out 0 n2 0 1", ".ends"]:
            s.x("type", "--delay", "4", line)
            s.x("key", "Return")
        s.x("mousemove", 395, 80, "click", 1)
        s.x("mousemove", 1300, 700)
        s.shot("19-spice-import.png", wait=0.8)
        s.close()

        # Online catalogs (needs network): JLCPCB enabled in an isolated config.
        cfg = home / ".config" / "electrocraft"
        cfg.mkdir(parents=True, exist_ok=True)
        (cfg / "settings.json").write_text(json.dumps({"catalogs": {"master": True, "enabled": ["jlcpcb"]}}))
        s = run("--sample", "divider", commands=[{"id": "parts.search", "args": {"query": "10k 0603", "provider": "jlcpcb"}}])
        s.x("mousemove", 380, 218, "click", 1)
        s.shot("34-parts-panel.png", wait=4.0)
        s.x("mousemove", 972, 541, "click", 1)
        s.x("mousemove", 1183, 38, "click", 1)
        s.x("mousemove", 1000, 860)
        s.shot("35-inspector-sourcing.png", wait=1.0)
        s.close()
        (cfg / "settings.json").unlink()

        datasheet = str(Path(args.binary).with_name("electrocraft-datasheet"))
        if Path(datasheet).exists():
            s = Session(datasheet, args.display, home, ("--example", "npn"))
            s.shot("20-datasheet-fit.png", wait=1.0)
            s.close()

        s = run("--sample", "astable")
        s.x("mousemove", 382, 56, "click", 1)
        s.x("mousemove", 140, 100, "click", 1)
        s.x("mousemove", 1300, 860)
        s.shot("14-dc-operating-point.png", wait=1.0)
        s.close()
    finally:
        xvfb.terminate()
        shutil.rmtree(home, ignore_errors=True)


if __name__ == "__main__":
    main()
