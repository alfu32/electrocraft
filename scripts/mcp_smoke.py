#!/usr/bin/env python3
"""Smoke test of `electrocraft --mcp` over stdio in a private Xvfb display
(needs Xvfb; ImageMagick `import` for --screenshot). Builds the manual's worked
example (5 V source, 330 ohm resistor, LED) through MCP tools and checks the
answers; the protocol stream must stay clean JSON."""

import argparse
import json
import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--binary", default=str(ROOT / "target" / "release" / "electrocraft"))
    ap.add_argument("--display", type=int, default=94)
    ap.add_argument("--screenshot")
    args = ap.parse_args()
    if not shutil.which("Xvfb"):
        raise SystemExit("Xvfb is required")
    xvfb = subprocess.Popen(["Xvfb", f":{args.display}", "-screen", "0", "1600x1000x24", "-noreset"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1.5)
    home = Path(tempfile.mkdtemp(prefix="electrocraft-mcp-"))
    env = dict(os.environ, DISPLAY=f":{args.display}", HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"))
    env.pop("WAYLAND_DISPLAY", None)
    proc = subprocess.Popen([args.binary, "--mcp", "--size", "1600x1000"], env=env, stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
    counter = [0]

    def rpc(method, params=None):
        counter[0] += 1
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": counter[0], "method": method, "params": params or {}}) + "\n")
        proc.stdin.flush()
        return json.loads(proc.stdout.readline())  # a stray print would break this

    def call(tool, **kwargs):
        result = rpc("tools/call", {"name": tool, "arguments": kwargs})["result"]
        return result["isError"], result["content"][0]["text"]

    try:
        init = rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "smoke", "version": "0"}})
        assert "get_manual" in init["result"]["instructions"]
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        tools = {t["name"] for t in rpc("tools/list")["result"]["tools"]}
        assert {"get_manual", "schematic_place", "document_describe", "sim_read"} <= tools
        uris = {r["uri"] for r in rpc("resources/list")["result"]["resources"]}
        assert {"manual://index", "manual://workflow", "manual://library"} <= uris
        assert "workflow" in call("get_manual")[1]
        error, text = call("schematic_place", **{"def": "core.resistor"}, x=0, y=0)
        assert error and "No document" in text, text
        assert call("file_new") == (False, "ok")
        for definition, x, y, rot in [("core.vdc", 0, 0, 0), ("core.resistor", 80, -40, 0), ("core.led", 160, 0, 1), ("core.ground", 0, 80, 0)]:
            error, text = call("schematic_place", **{"def": definition}, x=x, y=y, rot=rot)
            assert not error and text.startswith("Placed"), text
        assert not call("schematic_set_param", refdes="R1", name="R", value="330")[0]
        for points in [[[0, -40], [40, -40]], [[120, -40], [160, -40]], [[0, 40], [0, 80]], [[160, 40], [160, 80], [0, 80]]]:
            assert not call("schematic_wire", points=points)[0]
        assert call("schematic_delete_part", refdes="R9")[0]
        described = json.loads(call("document_describe")[1])
        assert described["problems"] == [] and described["unconnected_pins"] == [], described["problems"]
        time.sleep(2.5)
        state = json.loads(call("sim_read")[1])
        current = state["parts"]["D1"]["currents"][0]
        assert 0.008 < current < 0.011, state  # about 9.4 mA
        assert call("parts_search", query="10k")[0]  # catalogs are off by default
        path = home / "smoke.ecs"
        assert not call("file_save_path", path=str(path))[0] and path.exists()
        assert call("file_save_path", path=str(home / "smoke.txt"))[0]
        if args.screenshot:
            subprocess.run(["import", "-window", "root", args.screenshot], env=env, check=True)
        print("mcp smoke ok: LED current %.2f mA" % (current * 1e3))
    finally:
        proc.stdin.close()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        xvfb.terminate()
        shutil.rmtree(home, ignore_errors=True)


if __name__ == "__main__":
    main()
