//! Stable command ids and argument shapes: the single registry behind menus,
//! ribbon, palette and (next) an MCP server. `electrocraft --command-catalog`
//! prints it as JSON, as Meshcraft does.

use crate::analysis_ui::Which;
use crate::app::{Cmd, Tool};
use serde_json::{Value, json};

/// One registered command.
pub struct Info {
    pub id: &'static str,
    pub title: &'static str,
    /// JSON shape of the arguments (`{}` when none).
    pub args: Value,
}

fn info(id: &'static str, title: &'static str, args: Value) -> Info {
    Info { id, title, args }
}

/// Every command in a stable order.
pub fn catalog() -> Vec<Info> {
    let none = json!({});
    vec![
        info("file.new", "New schematic", none.clone()),
        info("file.open", "Open… (dialog)", none.clone()),
        info(
            "file.open_path",
            "Open a schematic file",
            json!({"path": "string"}),
        ),
        info(
            "file.open_sample",
            "Open a sample circuit",
            json!({"id": "led-switch | rc-lowpass | astable | counter | rectifier | rc-blocks | pic-blink | divider | hex-register"}),
        ),
        info(
            "file.save",
            "Save (an untitled document needs file.save_path)",
            none.clone(),
        ),
        info(
            "file.save_path",
            "Save the active document to a .ecs file",
            json!({"path": "string ending in .ecs"}),
        ),
        info(
            "file.save_all",
            "Save every modified document",
            none.clone(),
        ),
        info(
            "file.close",
            "Close a tab (the active one without index); asks to save changes",
            json!({"index": "optional tab number, from 0"}),
        ),
        info(
            "file.switch_tab",
            "Show a tab",
            json!({"index": "tab number, from 0"}),
        ),
        info("file.next_tab", "Show the next tab", none.clone()),
        info("file.save_as", "Save as… (dialog)", none.clone()),
        info(
            "file.import_spice",
            "Import SPICE models (dialog)",
            none.clone(),
        ),
        info(
            "file.export_spice",
            "Export SPICE netlist (dialog)",
            none.clone(),
        ),
        info(
            "file.components_folder",
            "Open the user components folder",
            none.clone(),
        ),
        info("edit.undo", "Undo", none.clone()),
        info("edit.redo", "Redo", none.clone()),
        info("edit.delete", "Delete the selection", none.clone()),
        info("edit.rotate", "Rotate the selection", none.clone()),
        info("edit.mirror", "Mirror the selection", none.clone()),
        info("edit.copy", "Copy", none.clone()),
        info("edit.cut", "Cut", none.clone()),
        info("edit.paste", "Paste at the pointer", none.clone()),
        info("edit.duplicate", "Duplicate", none.clone()),
        info("edit.select_all", "Select all", none.clone()),
        info(
            "schematic.place",
            "Place a part",
            json!({"def": "component id, e.g. core.resistor", "x": "number", "y": "number", "rot": "0..3"}),
        ),
        info(
            "schematic.wire",
            "Add a wire",
            json!({"points": "[[x, y], ...]"}),
        ),
        info(
            "schematic.set_param",
            "Set a part parameter",
            json!({"refdes": "string", "name": "string", "value": "engineering notation"}),
        ),
        info(
            "schematic.set_part_number",
            "Set a part number (known numbers apply their model parameters)",
            json!({"refdes": "string", "number": "e.g. 2N2222A, 74HC00, PIC16F84A"}),
        ),
        info(
            "schematic.set_program",
            "Set a microcontroller's program",
            json!({"refdes": "string", "code": "assembly source"}),
        ),
        info(
            "mcu.edit_program",
            "Open a microcontroller's program editor",
            json!({"refdes": "string"}),
        ),
        info(
            "schematic.delete_part",
            "Delete a part",
            json!({"refdes": "string"}),
        ),
        info(
            "schematic.label",
            "Add a net label (labels with the same name are connected; GND is ground)",
            json!({"name": "string", "x": "number", "y": "number"}),
        ),
        info(
            "schematic.text",
            "Add a text note",
            json!({"text": "string", "x": "number", "y": "number"}),
        ),
        info(
            "schematic.move_part",
            "Move a part to a position; wires attached to its pins stretch",
            json!({"refdes": "string", "x": "number", "y": "number"}),
        ),
        info(
            "schematic.orient_part",
            "Set a part's rotation (quarter turns clockwise) and optionally mirroring",
            json!({"refdes": "string", "rot": "0..3", "mirror": "optional boolean as true or false"}),
        ),
        info(
            "schematic.delete_wire",
            "Delete a wire by its id (ids are listed by document_describe)",
            json!({"wire_id": "wire id, integer"}),
        ),
        info(
            "schematic.select",
            "Select parts (for edit.copy, edit.rotate, ...); empty clears the selection",
            json!({"refdes": "optional designators, comma-separated"}),
        ),
        info(
            "scope.add",
            "Add a scope channel",
            json!({"expr": "formula over codes, e.g. out, R1.I, Q1.B.V, (in - out) / 1k", "name": "optional label"}),
        ),
        info(
            "scope.remove",
            "Remove scope channels with a formula",
            json!({"expr": "string"}),
        ),
        info("scope.clear", "Remove every scope channel", none.clone()),
        info("component.new", "New component (editor)", none.clone()),
        info(
            "component.edit",
            "Edit a part's component (editor)",
            json!({"refdes": "string"}),
        ),
        info(
            "component.enter",
            "Edit a part's inner schematic",
            json!({"refdes": "string"}),
        ),
        info(
            "component.exit",
            "Return to the enclosing schematic",
            none.clone(),
        ),
        info("tool.select", "Select tool", none.clone()),
        info("tool.wire", "Wire tool", none.clone()),
        info("tool.label", "Net label tool", none.clone()),
        info("tool.text", "Text tool", none.clone()),
        info("tool.probe", "Probe tool", none.clone()),
        info("tool.pan", "Pan tool", none.clone()),
        info(
            "tool.place",
            "Arm placement of a component",
            json!({"def": "component id"}),
        ),
        info(
            "sim.toggle_run",
            "Run or pause the live simulation",
            none.clone(),
        ),
        info("sim.step", "Advance one step", none.clone()),
        info("sim.reset", "Reset to the operating point", none.clone()),
        info(
            "sim.control",
            "Operate a control while running",
            json!({"refdes": "string", "state": "closed | pressed | position | on", "value": "number"}),
        ),
        info(
            "analysis.dc_op_annotate",
            "DC operating point, annotated on the schematic",
            none.clone(),
        ),
        info(
            "analysis.dialog",
            "Open an analysis dialog",
            json!({"kind": "op | dc | tran | ac"}),
        ),
        info(
            "analysis.run",
            "Run an analysis with the current settings",
            json!({"kind": "op | dc | tran | ac"}),
        ),
        info("view.fit", "Fit to view", none.clone()),
        info("view.zoom_in", "Zoom in", none.clone()),
        info("view.zoom_out", "Zoom out", none.clone()),
        info("view.grid", "Toggle the grid", none.clone()),
        info("view.voltage", "Toggle voltage colours", none.clone()),
        info("view.current", "Toggle current dots", none.clone()),
        info(
            "view.panel",
            "Toggle a panel",
            json!({"panel": "components | inspector | scopes"}),
        ),
        info(
            "parts.open",
            "Show or hide the Parts panel (online catalogs)",
            none.clone(),
        ),
        info(
            "parts.search",
            "Search the enabled online catalogs (Parts panel)",
            json!({"query": "string", "provider": "optional: jlcpcb | digikey | mouser | tme | farnell (default all enabled)"}),
        ),
        info(
            "sourcing.set",
            "Set sourcing fields of parts",
            json!({"refdes": "designators, comma-separated", "mpn": "optional", "manufacturer": "optional", "preferred_vendor": "optional: jlcpcb | digikey | mouser | tme | farnell | empty"}),
        ),
        info(
            "sourcing.find_all",
            "Look parts up at every enabled vendor by manufacturer part number",
            json!({"refdes": "optional designators, comma-separated (default: selected parts without vendors)"}),
        ),
        info(
            "sourcing.refresh",
            "Refresh a part's vendor entries",
            json!({"refdes": "string"}),
        ),
        info(
            "sourcing.clear",
            "Clear a part's sourcing",
            json!({"refdes": "string"}),
        ),
        info(
            "bom.export",
            "Export the bill of materials (dialog)",
            none.clone(),
        ),
        info(
            "bom.export_path",
            "Export the bill of materials as CSV, plus one file per preferred vendor",
            json!({"path": "string"}),
        ),
        info("app.home", "Start screen", none.clone()),
        info("app.settings", "Settings", none.clone()),
        info("app.mcp_start", "Start MCP server", none.clone()),
        info("app.mcp_stop", "Stop MCP server", none.clone()),
        info(
            "view.dot_speed",
            "Current-dot speed factor",
            json!({"value": "0.05 to 4 (default 0.35)"}),
        ),
        info("app.palette", "Command palette", none.clone()),
        info("app.shortcuts", "Keyboard shortcuts", none.clone()),
        info("app.about", "About", none.clone()),
        info("app.quit", "Quit", none),
    ]
}

fn which(args: &Value) -> Result<Which, String> {
    match args["kind"].as_str() {
        Some("op") => Ok(Which::Op),
        Some("dc") => Ok(Which::Dc),
        Some("tran") => Ok(Which::Tran),
        Some("ac") => Ok(Which::Ac),
        other => Err(format!("unknown analysis kind {other:?}")),
    }
}

fn s(args: &Value, k: &str) -> Result<String, String> {
    args[k]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("missing string argument {k}"))
}

fn n(args: &Value, k: &str) -> Result<f64, String> {
    args[k]
        .as_f64()
        .ok_or_else(|| format!("missing number argument {k}"))
}

/// Build a command from its id and JSON arguments.
pub fn from_id(id: &str, args: &Value) -> Result<Cmd, String> {
    Ok(match id {
        "file.new" => Cmd::New,
        "file.open" => Cmd::Open,
        "file.open_path" => Cmd::OpenPath(s(args, "path")?.into()),
        "file.open_sample" => {
            let want = s(args, "id")?;
            let sample = crate::samples::all()
                .into_iter()
                .find(|x| x.id == want)
                .ok_or_else(|| format!("unknown sample {want}"))?;
            Cmd::OpenSample(sample.id)
        }
        "file.save" => Cmd::Save,
        "file.save_path" => Cmd::SavePath(s(args, "path")?.into()),
        "file.save_all" => Cmd::SaveAll,
        "file.close" => Cmd::CloseTab(args["index"].as_u64().map(|k| k as usize)),
        "file.switch_tab" => {
            Cmd::SwitchTab(args["index"].as_u64().ok_or("missing tab index")? as usize)
        }
        "file.next_tab" => Cmd::NextTab,
        "file.save_as" => Cmd::SaveAs,
        "file.import_spice" => Cmd::ImportSpice,
        "file.export_spice" => Cmd::ExportSpice,
        "file.components_folder" => Cmd::OpenComponentsFolder,
        "app.datasheet_tool" => Cmd::DatasheetTool,
        "edit.undo" => Cmd::Undo,
        "edit.redo" => Cmd::Redo,
        "edit.delete" => Cmd::Delete,
        "edit.rotate" => Cmd::Rotate,
        "edit.mirror" => Cmd::Mirror,
        "edit.copy" => Cmd::Copy,
        "edit.cut" => Cmd::Cut,
        "edit.paste" => Cmd::Paste,
        "edit.duplicate" => Cmd::Duplicate,
        "edit.select_all" => Cmd::SelectAll,
        "schematic.place" => Cmd::Place {
            def: s(args, "def")?,
            at: [n(args, "x")?, n(args, "y")?],
            rot: args["rot"].as_u64().unwrap_or(0) as u8,
        },
        "schematic.wire" => {
            let pts = args["points"].as_array().ok_or("points must be an array")?;
            let pts: Vec<[f64; 2]> = pts
                .iter()
                .map(|p| [p[0].as_f64().unwrap_or(0.0), p[1].as_f64().unwrap_or(0.0)])
                .collect();
            // Wires are orthogonal: a diagonal pair gets a horizontal-first elbow.
            let mut out: Vec<[f64; 2]> = Vec::new();
            for p in pts {
                match out.last().copied() {
                    Some(last) if last[0] != p[0] && last[1] != p[1] => {
                        out.extend(ecs_schematic::route(last, p, true).into_iter().skip(1));
                    }
                    _ => out.push(p),
                }
            }
            Cmd::AddWire(out)
        }
        "schematic.set_param" => Cmd::SetParam {
            refdes: s(args, "refdes")?,
            name: s(args, "name")?,
            value: s(args, "value")?,
        },
        "schematic.delete_part" => Cmd::DeletePart(s(args, "refdes")?),
        "schematic.label" => Cmd::Label {
            name: s(args, "name")?,
            at: [n(args, "x")?, n(args, "y")?],
        },
        "schematic.text" => Cmd::Text {
            text: s(args, "text")?,
            at: [n(args, "x")?, n(args, "y")?],
        },
        "schematic.move_part" => Cmd::MovePart {
            refdes: s(args, "refdes")?,
            at: [n(args, "x")?, n(args, "y")?],
        },
        "schematic.orient_part" => Cmd::OrientPart {
            refdes: s(args, "refdes")?,
            rot: args["rot"].as_u64().ok_or("missing integer argument rot")? as u8,
            mirror: args["mirror"].as_bool(),
        },
        "schematic.delete_wire" => Cmd::DeleteWire(
            args["wire_id"]
                .as_u64()
                .ok_or("missing integer argument wire_id")?,
        ),
        "schematic.select" => Cmd::SelectParts(args["refdes"].as_str().unwrap_or("").to_string()),
        "schematic.set_part_number" => Cmd::SetPartNumber {
            refdes: s(args, "refdes")?,
            number: s(args, "number")?,
        },
        "schematic.set_program" => Cmd::SetProgram {
            refdes: s(args, "refdes")?,
            code: s(args, "code")?,
        },
        "mcu.edit_program" => Cmd::EditProgram(s(args, "refdes")?),
        "scope.clear" | "schematic.clear_probes" => Cmd::ClearProbes,
        "scope.add" => {
            let expr = s(args, "expr")?;
            Cmd::ScopeAdd {
                name: args["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&expr)
                    .to_string(),
                expr,
            }
        }
        "scope.remove" => Cmd::ScopeRemove(s(args, "expr")?),
        "component.new" => Cmd::NewComponent,
        "component.edit" => Cmd::EditComponent(s(args, "refdes")?),
        "component.enter" => Cmd::EnterComponent(s(args, "refdes")?),
        "component.exit" => Cmd::ExitComponent,
        "tool.select" => Cmd::Tool(Tool::Select),
        "tool.wire" => Cmd::Tool(Tool::Wire),
        "tool.label" => Cmd::Tool(Tool::Label),
        "tool.text" => Cmd::Tool(Tool::Text),
        "tool.probe" => Cmd::Tool(Tool::Probe),
        "tool.pan" => Cmd::Tool(Tool::Pan),
        "tool.place" => Cmd::Tool(Tool::Place {
            def: s(args, "def")?,
            rot: 0,
            mirror: false,
        }),
        "sim.toggle_run" => Cmd::ToggleRun,
        "sim.step" => Cmd::Step,
        "sim.reset" => Cmd::Reset,
        "sim.control" => Cmd::Control {
            refdes: s(args, "refdes")?,
            state: s(args, "state")?,
            value: n(args, "value")?,
        },
        "analysis.dc_op_annotate" => Cmd::DcOp,
        "analysis.dialog" => Cmd::Analysis(which(args)?),
        "analysis.run" => Cmd::RunAnalysis(which(args)?),
        "view.fit" => Cmd::Fit,
        "view.zoom_in" => Cmd::ZoomIn,
        "view.zoom_out" => Cmd::ZoomOut,
        "view.grid" => Cmd::ToggleGrid,
        "view.voltage" => Cmd::ToggleVolts,
        "view.current" => Cmd::ToggleDots,
        "view.panel" => Cmd::TogglePanel(match args["panel"].as_str() {
            Some("components") => "components",
            Some("inspector") => "inspector",
            _ => "scopes",
        }),
        "parts.open" => Cmd::PartsPanel,
        "parts.search" => Cmd::PartsSearch {
            query: s(args, "query")?,
            provider: args["provider"]
                .as_str()
                .filter(|p| !p.is_empty())
                .map(str::to_string),
        },
        "sourcing.set" => Cmd::SetSourcing {
            refdes: s(args, "refdes")?,
            mpn: args["mpn"].as_str().map(str::to_string),
            manufacturer: args["manufacturer"].as_str().map(str::to_string),
            preferred_vendor: args["preferred_vendor"].as_str().map(str::to_string),
        },
        "sourcing.find_all" => {
            Cmd::FindAtVendors(args["refdes"].as_str().unwrap_or("").to_string())
        }
        "sourcing.refresh" => Cmd::RefreshSourcing(s(args, "refdes")?),
        "sourcing.clear" => Cmd::ClearSourcing(s(args, "refdes")?),
        "bom.export" => Cmd::ExportBom,
        "bom.export_path" => Cmd::ExportBomPath(s(args, "path")?.into()),
        "app.home" => Cmd::Home,
        "app.settings" => Cmd::Settings,
        "app.mcp_start" => Cmd::McpStart,
        "app.mcp_stop" => Cmd::McpStop,
        "view.dot_speed" => Cmd::DotSpeed(n(args, "value")?),
        "app.palette" => Cmd::Palette,
        "app.shortcuts" => Cmd::Shortcuts,
        "app.about" => Cmd::About,
        "app.quit" => Cmd::Quit,
        other => return Err(format!("unknown command {other}")),
    })
}

/// The catalog as JSON (for `--command-catalog`).
pub fn catalog_json() -> String {
    let v: Vec<Value> = catalog()
        .into_iter()
        .map(|i| json!({"id": i.id, "title": i.title, "args": i.args}))
        .collect();
    serde_json::to_string_pretty(
        &json!({"app": "electrocraft", "version": env!("CARGO_PKG_VERSION"), "commands": v}),
    )
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_entry_builds_a_command() {
        let mut ids = std::collections::BTreeSet::new();
        for i in catalog() {
            assert!(ids.insert(i.id), "duplicate id {}", i.id);
            let args = json!({"path": "x.ecs", "id": "astable", "def": "core.resistor", "x": 0.0, "y": 0.0, "points": [[0, 0], [20, 0]], "refdes": "R1", "name": "R", "value": "1k", "state": "closed", "kind": "tran", "panel": "scopes", "expr": "R1.I", "index": 0, "number": "2N2222A", "code": "nop", "query": "10k", "text": "note", "wire_id": 1, "rot": 1});
            let args = match i.id {
                "sim.control" => json!({"refdes": "S1", "state": "closed", "value": 1.0}),
                "view.dot_speed" => json!({"value": 0.5}),
                _ => args,
            };
            from_id(i.id, &args).unwrap_or_else(|e| panic!("{}: {e}", i.id));
        }
        assert!(from_id("nope", &json!({})).is_err());
    }
}
