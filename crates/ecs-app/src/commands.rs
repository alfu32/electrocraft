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
            json!({"id": "led-switch | rc-lowpass | astable | counter | rectifier | rc-blocks | divider"}),
        ),
        info("file.save", "Save", none.clone()),
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
            "schematic.delete_part",
            "Delete a part",
            json!({"refdes": "string"}),
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
        info("app.home", "Start screen", none.clone()),
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
            Cmd::AddWire(
                pts.iter()
                    .map(|p| [p[0].as_f64().unwrap_or(0.0), p[1].as_f64().unwrap_or(0.0)])
                    .collect(),
            )
        }
        "schematic.set_param" => Cmd::SetParam {
            refdes: s(args, "refdes")?,
            name: s(args, "name")?,
            value: s(args, "value")?,
        },
        "schematic.delete_part" => Cmd::DeletePart(s(args, "refdes")?),
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
        "app.home" => Cmd::Home,
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
            let args = json!({"path": "x.ecs", "id": "astable", "def": "core.resistor", "x": 0.0, "y": 0.0, "points": [[0, 0], [20, 0]], "refdes": "R1", "name": "R", "value": "1k", "state": "closed", "kind": "tran", "panel": "scopes", "expr": "R1.I"});
            let args = if i.id == "sim.control" {
                json!({"refdes": "S1", "state": "closed", "value": 1.0})
            } else {
                args
            };
            from_id(i.id, &args).unwrap_or_else(|e| panic!("{}: {e}", i.id));
        }
        assert!(from_id("nope", &json!({})).is_err());
    }
}
