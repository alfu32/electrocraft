//! The MCP server inside the app (`electrocraft --mcp`, spec 9.6, D14).
//!
//! The MCP client launches the app; the window opens as usual and the client
//! drives it over stdio. A reader thread speaks the protocol (`ecs-mcp`) and
//! forwards every tool call to the UI thread, which runs it as a registry
//! command between frames, so the user sees each step and the document stays
//! single-threaded. Nothing else may write to stdout: the protocol owns the
//! original stdout, and fd 1 is pointed at stderr for libraries that print.
//!
//! Tools: every registry command except those that open a dialog or window
//! (they need the user), plus read-only `document_describe`, `sim_read` and
//! `parts_results`, plus `get_manual`. Consent for online catalogs is not a
//! command and so can never be granted through MCP.

use crate::app::{App, Cmd};
use crate::catalog;
use crate::commands;
use ecs_mcp::{Backend, Section, Server, ToolDef};
use serde_json::{Value, json};
use std::io::Write;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::time::{Duration, Instant};

const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

/// A request for the UI thread.
pub enum Job {
    Call {
        tool: String,
        args: Value,
        reply: Sender<Result<String, String>>,
        deadline: Instant,
    },
    /// The client closed the connection.
    Closed,
}

/// Registry commands that are not offered as tools: they open dialogs or
/// windows, depend on the pointer, or end the application.
fn exposed(id: &str) -> bool {
    const HIDDEN: [&str; 26] = [
        "file.open",
        "file.save_as",
        "file.import_spice",
        "file.export_spice",
        "file.components_folder",
        "bom.export",
        "app.quit",
        "app.datasheet_tool",
        "app.palette",
        "app.shortcuts",
        "app.about",
        "app.settings",
        "app.mcp_start",
        "app.mcp_stop",
        "app.home",
        "analysis.dialog",
        "component.new",
        "component.edit",
        "mcu.edit_program",
        "edit.paste",
        "view.panel",
        "view.dot_speed",
        "view.grid",
        "view.voltage",
        "view.current",
        "parts.open",
    ];
    !HIDDEN.contains(&id) && !id.starts_with("tool.")
}

/// Tool name of a command id.
pub fn tool_name(id: &str) -> String {
    id.replace('.', "_")
}

fn topic_of(id: &str) -> &'static str {
    match id {
        "schematic.set_param" | "schematic.set_part_number" | "sim.control" => "parameters",
        "schematic.set_program" => "microcontrollers",
        _ => match id.split('.').next().unwrap_or("") {
            "file" => "files",
            "edit" | "schematic" => "schematic",
            "mcu" => "microcontrollers",
            "scope" => "scope",
            "component" => "components",
            "sim" | "view" => "simulation",
            "analysis" => "analyses",
            "parts" | "sourcing" | "bom" => "catalogs",
            _ => "overview",
        },
    }
}

/// The read-only tools that are not commands.
fn read_tools() -> Vec<ToolDef> {
    let def = |name: &str, title: &str, description: &str, schema: Value| ToolDef {
        name: name.into(),
        title: title.into(),
        description: description.into(),
        input_schema: schema,
        read_only: true,
    };
    vec![
        def(
            "document_describe",
            "Describe the document",
            "Describe the active schematic: every part (designator, definition, position, rotation, effective parameters, pins with world coordinates and net names), wires with ids and nets, labels, notes, scope channels, selection and the problems found (unconnected pins, unknown parts). Use it to find pin positions before wiring and to check the circuit before running. For details, call get_manual with topic schematic.",
            json!({"type": "object", "properties": {"refdes": {"type": "string", "description": "Optional: only these designators, comma-separated"}}, "additionalProperties": false}),
        ),
        def(
            "sim_read",
            "Read live simulation values",
            "Read the live simulation: running state, simulated time, errors, voltage of every net, and per part the branch currents and named probes (and the DC operating point when it was annotated). Values appear after the simulation has stepped on the current netlist; call again after a moment following an edit. For details, call get_manual with topic simulation.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ),
        def(
            "parts_results",
            "Read part catalog results",
            "Read the results of the last parts_search (and whether it is still running): per provider the parts found with order codes, stock, prices and normalized parameters, or the error. For details, call get_manual with topic catalogs.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ),
    ]
}

/// Every tool offered, in a stable order.
pub fn tool_defs() -> Vec<ToolDef> {
    let mut tools: Vec<ToolDef> = commands::catalog()
        .into_iter()
        .filter(|i| exposed(i.id))
        .map(|i| ToolDef {
            name: tool_name(i.id),
            title: i.title.to_string(),
            description: format!(
                "{}. For details, call get_manual with topic {}.",
                i.title.trim_end_matches('.'),
                topic_of(i.id)
            ),
            input_schema: ecs_mcp::schema_from_args(&i.args),
            read_only: false,
        })
        .collect();
    tools.extend(read_tools());
    tools
}

/// The command id behind a tool name.
fn command_id(tool: &str) -> Option<&'static str> {
    static IDS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    IDS.get_or_init(|| {
        commands::catalog()
            .into_iter()
            .map(|i| i.id)
            .filter(|id| exposed(id))
            .collect()
    })
    .iter()
    .copied()
    .find(|id| tool_name(id) == tool)
}

/// What the protocol thread uses to reach the UI thread and the manual.
#[derive(Clone)]
pub struct AppBackend {
    tx: SyncSender<Job>,
    tools: Arc<Vec<ToolDef>>,
    ctx: egui::Context,
}

impl AppBackend {
    pub fn new(tx: SyncSender<Job>, ctx: egui::Context) -> Self {
        Self {
            tx,
            tools: Arc::new(tool_defs()),
            ctx,
        }
    }
}

impl Backend for AppBackend {
    fn info(&self) -> (String, String) {
        ("electrocraft".into(), env!("CARGO_PKG_VERSION").into())
    }
    fn tools(&self) -> Vec<ToolDef> {
        self.tools.as_ref().clone()
    }
    fn call(&mut self, tool: &str, args: &Value) -> Result<String, String> {
        let (reply, wait) = channel();
        self.tx
            .try_send(Job::Call {
                tool: tool.into(),
                args: args.clone(),
                reply,
                deadline: Instant::now() + TOOL_TIMEOUT,
            })
            .map_err(|error| match error {
                std::sync::mpsc::TrySendError::Full(_) => "Electrocraft is busy".to_string(),
                std::sync::mpsc::TrySendError::Disconnected(_) => "MCP server stopped".to_string(),
            })?;
        self.ctx.request_repaint();
        match wait.recv_timeout(TOOL_TIMEOUT) {
            Ok(r) => r,
            Err(_) => Err("Electrocraft did not answer in time (is a dialog open?)".into()),
        }
    }
    fn sections(&self) -> Vec<Section> {
        static SECTIONS: std::sync::OnceLock<Vec<Section>> = std::sync::OnceLock::new();
        SECTIONS.get_or_init(build_sections).clone()
    }
}

/// Manual sections that come from the running build.
pub fn build_sections() -> Vec<Section> {
    let mut commands_md = String::from(
        "Every tool, with its arguments (JSON). Names are the command id with dots as underscores; the id is in parentheses.\n\n",
    );
    for t in tool_defs() {
        let id = command_id(&t.name).unwrap_or("");
        let props = t.input_schema["properties"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let required: Vec<&str> = t.input_schema["required"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let args: Vec<String> = props
            .iter()
            .map(|(k, v)| {
                format!(
                    "{k}{}: {}",
                    if required.contains(&k.as_str()) {
                        ""
                    } else {
                        "?"
                    },
                    v["description"].as_str().unwrap_or("")
                )
            })
            .collect();
        let id = if id.is_empty() {
            String::new()
        } else {
            format!(" ({id})")
        };
        commands_md += &format!(
            "- `{}`{id}{}: {}{}\n",
            t.name,
            if t.read_only { " [read-only]" } else { "" },
            t.title,
            if args.is_empty() {
                String::new()
            } else {
                format!("; arguments: {}", args.join("; "))
            }
        );
    }
    commands_md += "\n`get_manual` takes `topic` or `query`. Arguments marked `?` are optional.\n";

    let cat = catalog::Catalog::builtin();
    let mut lib = String::from(
        "Component definitions for `schematic_place` (`def`), by category. Each line: id, name, pins, then parameters (`name` = default unit). Place a part, then read its real pin positions from the answer or `document_describe`.\n",
    );
    for category in cat.categories() {
        lib += &format!("\n## {category}\n\n");
        for d in cat.category(&category) {
            let pins: Vec<&str> = d.pins.iter().map(|p| p.name.as_str()).collect();
            let params: Vec<String> = d
                .params
                .iter()
                .map(|p| match &p.kind {
                    ecs_model::ParamKind::Enum(names) => {
                        format!("{} in [{}]", p.name, names.join("|"))
                    }
                    _ => format!(
                        "{} = {}{}",
                        p.name,
                        ecs_units::format_digits(p.default, "", 4).replace(' ', ""),
                        if p.unit.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", p.unit)
                        }
                    ),
                })
                .collect();
            lib += &format!(
                "- `{}` {}: pins {}{}\n",
                d.id,
                d.meta.name,
                pins.join(", "),
                if params.is_empty() {
                    String::new()
                } else {
                    format!("; {}", params.join(", "))
                }
            );
        }
    }

    let mut numbers = String::from(
        "Part numbers that `schematic_set_part_number` knows (they set the model parameters), by component definition.\n\n",
    );
    let mut by_def: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for k in ecs_lib::parts::known_parts() {
        by_def
            .entry(k.def.as_str())
            .or_default()
            .push(k.number.as_str());
    }
    for (def, list) in by_def {
        numbers += &format!("- `{def}`: {}\n", list.join(", "));
    }

    let mut scripting = String::from(
        "User components are JavaScript files (ES2020 on QuickJS, sandboxed) in the user components folder; they are hot-reloaded when saved. A script runs once to describe the component: pins, parameters, and equations as plain functions (compiled to expressions, never run in the solver) or `e` templates. TypeScript declarations of the whole API:\n\n```ts\n",
    );
    scripting += ecs_script::DTS;
    scripting += "\n```\n\nReference components (each is a complete working example):\n\n";
    for (name, src) in ecs_script::REFERENCE {
        scripting += &format!("### {name}\n\n```js\n{src}\n```\n\n");
    }

    vec![
        Section::new(
            "commands",
            "Tools and commands",
            "Every tool with its arguments",
            commands_md,
        ),
        Section::new(
            "library",
            "Component library",
            "Definition ids, pins and parameters for schematic_place",
            lib,
        ),
        Section::new(
            "part-numbers",
            "Known part numbers",
            "Part numbers whose model parameters are built in",
            numbers,
        ),
        Section::new(
            "scripting-api",
            "JavaScript components API",
            "TypeScript declarations and reference components",
            scripting,
        ),
    ]
}

/// Take the process's real stdout for the protocol and point fd 1 at stderr,
/// so a library printing to stdout cannot corrupt the stream.
pub fn take_stdout() -> Box<dyn Write + Send> {
    #[cfg(unix)]
    {
        use std::os::fd::FromRawFd;
        unsafe extern "C" {
            fn dup(fd: i32) -> i32;
            fn dup2(from: i32, to: i32) -> i32;
        }
        // SAFETY: plain POSIX descriptor calls on the standard descriptors; the
        // duplicate is owned by the returned File.
        unsafe {
            let keep = dup(1);
            if keep >= 0 {
                dup2(2, 1);
                return Box::new(std::fs::File::from_raw_fd(keep));
            }
        }
    }
    Box::new(std::io::stdout())
}

/// Start the protocol thread; the UI thread polls the returned receiver.
pub fn start(ctx: egui::Context, out: Box<dyn Write + Send>) -> Receiver<Job> {
    let (tx, rx) = sync_channel(32);
    std::thread::Builder::new()
        .name("mcp".into())
        .spawn(move || {
            let mut server = Server::new(AppBackend::new(tx.clone(), ctx.clone()));
            let stdin = std::io::stdin();
            let _ = server.serve(stdin.lock(), out);
            let _ = tx.send(Job::Closed);
            ctx.request_repaint();
        })
        .expect("mcp thread");
    rx
}

fn looks_failed(status: &str) -> bool {
    [
        "No part",
        "No wire",
        "Unknown component",
        "Could not",
        "Schematics are saved",
        "Set a manufacturer",
        "Online catalogs are off",
        "Type a part",
    ]
    .iter()
    .any(|p| status.starts_with(p))
}

impl App {
    /// Run the tool calls that arrived (each frame while serving MCP).
    pub fn poll_mcp(&mut self, ctx: &egui::Context) {
        self.mcp_network.poll();
        for _ in 0..32 {
            let stdio = self.mcp.as_ref().and_then(|rx| rx.try_recv().ok());
            let network = self.mcp_network.try_job();
            if stdio.is_none() && network.is_none() {
                break;
            }
            for job in stdio.into_iter().chain(network) {
                match job {
                    Job::Call {
                        tool,
                        args,
                        reply,
                        deadline,
                    } => {
                        if Instant::now() < deadline {
                            let result = self.mcp_call(&tool, &args, ctx);
                            let _ = reply.send(result);
                        }
                    }
                    Job::Closed => {
                        self.mcp = None;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
    }

    fn mcp_call(
        &mut self,
        tool: &str,
        args: &Value,
        ctx: &egui::Context,
    ) -> Result<String, String> {
        match tool {
            "document_describe" => return Ok(self.describe(args["refdes"].as_str())),
            "sim_read" => return Ok(self.sim_read()),
            "parts_results" => return Ok(self.parts_results()),
            _ => {}
        }
        let id = command_id(tool).ok_or_else(|| format!("Unknown tool {tool}"))?;
        let is_file = id.starts_with("file.") || id.starts_with("app.");
        if !self.has_doc && !is_file {
            return Err(
                "No document is open: call file_new, file_open_path or file_open_sample first"
                    .into(),
            );
        }
        if id == "file.save" && self.path.is_none() && self.stack.is_empty() {
            return Err(
                "The document has no file yet: call file_save_path with a .ecs path".into(),
            );
        }
        if id == "bom.export_path"
            && let Some(p) = args["path"].as_str()
            && !p.to_lowercase().ends_with(".csv")
        {
            return Err("The bill of materials is written as a .csv file".into());
        }
        let cmd: Cmd = commands::from_id(id, args)?;
        let editing =
            id.starts_with("schematic.") || id.starts_with("edit.") || id == "sourcing.set";
        let before = (
            self.rev,
            self.doc.parts.len(),
            self.doc.wires.len(),
            self.doc.labels.len(),
        );
        let was_running = self.sim_running;
        self.status = format!("Agent: {tool}");
        let before_status = self.status.clone();
        self.run(cmd, ctx);
        let status = if self.status == before_status {
            String::new()
        } else {
            self.status.clone()
        };
        if looks_failed(&status) {
            return Err(status);
        }
        let mut out = if status.is_empty() {
            "ok".to_string()
        } else {
            status
        };
        let after = (
            self.rev,
            self.doc.parts.len(),
            self.doc.wires.len(),
            self.doc.labels.len(),
        );
        if self.has_doc && (before.1 != after.1 || before.2 != after.2 || before.3 != after.3) {
            out += &format!(
                "\nparts {} -> {}, wires {} -> {}, labels {} -> {}",
                before.1, after.1, before.2, after.2, before.3, after.3
            );
        } else if editing && before.0 == after.0 {
            out += "\n(the document did not change)";
        }
        // The snapshot only updates next frame: say what the command makes of it.
        match id {
            "sim.toggle_run" => {
                out += &format!(
                    "\nsimulation {}",
                    if was_running { "paused" } else { "running" }
                )
            }
            "sim.step" => out += "\nsimulation paused (one step taken)",
            _ => {}
        }
        if id == "schematic.place" {
            let refs: Vec<String> = self
                .selection
                .iter()
                .filter_map(|i| match i {
                    ecs_schematic::Item::Part(pid) => self.doc.part(*pid).map(|p| p.refdes.clone()),
                    _ => None,
                })
                .collect();
            if !refs.is_empty() {
                out += &format!("\n{}", self.describe(Some(&refs.join(","))));
            }
        }
        Ok(out)
    }

    fn describe(&mut self, only: Option<&str>) -> String {
        if !self.has_doc {
            return json!({"open": false, "hint": "no document is open"}).to_string();
        }
        let only: Option<Vec<&str>> = only.map(|l| {
            l.split(',')
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .collect()
        });
        let built = catalog::build(&self.cat, &self.doc);
        let parts: Vec<Value> = self
            .doc
            .parts
            .iter()
            .filter(|p| only.as_ref().is_none_or(|o| o.contains(&p.refdes.as_str())))
            .map(|p| {
                let def = self.cat.def(&p.def);
                let nets = built.pins.get(&p.id);
                let pins: Vec<Value> = p
                    .pins()
                    .iter()
                    .enumerate()
                    .map(|(k, (name, at))| json!({"name": name, "at": at, "net": nets.and_then(|n| n.get(k)).cloned().unwrap_or_default()}))
                    .collect();
                let params: serde_json::Map<String, Value> = def
                    .map(|d| d.params.iter().map(|q| (q.name.clone(), json!(catalog::param_text(p, d, &q.name).replace(' ', "")))).collect())
                    .unwrap_or_default();
                let mut v = json!({"refdes": p.refdes, "def": p.def, "name": def.map(|d| d.meta.name.clone()), "pos": p.pos, "rot": p.rot, "mirror": p.mirror, "params": params, "pins": pins});
                if !p.part_number.is_empty() {
                    v["part_number"] = json!(p.part_number);
                }
                if !p.sourcing.is_empty() {
                    v["sourcing"] = serde_json::to_value(&p.sourcing).unwrap_or(Value::Null);
                }
                v
            })
            .collect();
        if only.is_some() {
            return json!({"parts": parts}).to_string();
        }
        let selected: Vec<String> = self
            .selection
            .iter()
            .filter_map(|i| match i {
                ecs_schematic::Item::Part(id) => self.doc.part(*id).map(|p| p.refdes.clone()),
                _ => None,
            })
            .collect();
        let unconnected: Vec<String> = built
            .unconnected
            .iter()
            .filter_map(|(id, k)| {
                let p = self.doc.part(*id)?;
                Some(format!(
                    "{}.{}",
                    p.refdes,
                    p.pins().get(*k).map(|x| x.0.clone()).unwrap_or_default()
                ))
            })
            .collect();
        // Fresh from the current document (the app's own list lags one frame).
        let mut problems = built.problems.clone();
        for pin in &unconnected {
            problems.push(format!("{pin} is not connected"));
        }
        problems.extend(self.local_problems.iter().cloned());
        problems.sort();
        problems.dedup();
        self.sync_sim(false);
        serde_json::to_string_pretty(&json!({
            "title": self.title(),
            "path": self.path.as_ref().map(|p| p.display().to_string()),
            "unsaved": self.modified(),
            "running": self.sim_running,
            "parts": parts,
            "wires": self.doc.wires.iter().map(|w| json!({"id": w.id, "points": w.points, "net": built.wires.get(&w.id)})).collect::<Vec<_>>(),
            "labels": self.doc.labels.iter().map(|l| json!({"id": l.id, "name": l.name, "at": l.at})).collect::<Vec<_>>(),
            "notes": self.doc.notes.iter().map(|n| json!({"text": n.text, "at": n.at})).collect::<Vec<_>>(),
            "scope": self.doc.scope.iter().map(|c| json!({"name": c.name, "expr": c.expr})).collect::<Vec<_>>(),
            "selection": selected,
            "unconnected_pins": unconnected,
            "problems": problems,
            "simulation": {"error": self.sim_error, "warnings": self.sim_warnings, "note": "from the last simulation snapshot; it follows edits after a moment"},
        }))
        .unwrap_or_default()
    }

    fn sim_read(&mut self) -> String {
        if !self.has_doc {
            return json!({"open": false, "hint": "no document is open"}).to_string();
        }
        self.sync_sim(false);
        let mut parts = serde_json::Map::new();
        for p in &self.doc.parts {
            let Some(currents) = self.live.part_i.get(&p.id) else {
                continue;
            };
            let mut v = json!({"currents": currents});
            if let (Some(def), Some(vals)) = (self.cat.def(&p.def), self.live.part_probe.get(&p.id))
            {
                let probes: serde_json::Map<String, Value> = def
                    .probes
                    .iter()
                    .zip(vals)
                    .map(|(d, x)| (d.name.clone(), json!(x)))
                    .collect();
                v["probes"] = Value::Object(probes);
            }
            parts.insert(p.refdes.clone(), v);
        }
        let mut out = json!({
            "running": self.sim_running,
            "time": self.sim_time,
            "error": self.sim_error,
            "warnings": self.sim_warnings,
            "nets": self.live.net_v,
            "parts": parts,
        });
        if self.live.net_v.is_empty() {
            out["note"] = json!(
                "no live values yet: the simulation is paused or rebuilding after an edit; call sim_toggle_run, wait a moment and read again"
            );
        }
        if let Some((nets, part_i)) = &self.annotate {
            let by_ref: serde_json::Map<String, Value> = part_i
                .iter()
                .filter_map(|(id, v)| self.doc.part(*id).map(|p| (p.refdes.clone(), json!(v))))
                .collect();
            out["dc_op"] = json!({"nets": nets, "parts": by_ref});
        }
        serde_json::to_string_pretty(&out).unwrap_or_default()
    }

    fn parts_results(&mut self) -> String {
        let results: Vec<Value> = self
            .parts
            .results
            .iter()
            .map(|(p, r)| match r {
                Ok(list) => json!({"provider": p.key(), "attribution": p.attribution(), "count": list.len(), "parts": list.iter().take(25).collect::<Vec<_>>()}),
                Err(e) => json!({"provider": p.key(), "error": e}),
            })
            .collect();
        json!({"pending": !self.parts.pending.is_empty(), "query": self.parts.text, "results": results}).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_come_from_the_registry_and_hide_dialogs() {
        let tools = tool_defs();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        for want in [
            "schematic_place",
            "schematic_wire",
            "schematic_set_param",
            "sim_toggle_run",
            "file_open_path",
            "file_save_path",
            "scope_add",
            "parts_search",
            "sourcing_set",
            "bom_export_path",
            "document_describe",
            "sim_read",
            "parts_results",
        ] {
            assert!(names.contains(&want), "{want} missing");
        }
        for hidden in [
            "file_open",
            "file_save_as",
            "app_quit",
            "bom_export",
            "tool_wire",
            "edit_paste",
            "parts_open",
        ] {
            assert!(!names.contains(&hidden), "{hidden} must not be a tool");
        }
        // No consent tool, ever.
        assert!(
            !names
                .iter()
                .any(|n| n.contains("consent") || n.contains("enable"))
        );
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
        for t in &tools {
            assert!(
                t.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "{}",
                t.name
            );
            assert!(
                t.description.contains("get_manual"),
                "{} does not point to the manual",
                t.name
            );
            assert_eq!(t.input_schema["type"], "object");
        }
    }

    #[test]
    fn every_exposed_command_builds_from_its_schema_example() {
        // Arguments generated from each tool's own schema must build a command.
        for i in commands::catalog().into_iter().filter(|i| exposed(i.id)) {
            let mut args = serde_json::Map::new();
            for (k, v) in ecs_mcp::schema_from_args(&i.args)["properties"]
                .as_object()
                .unwrap()
            {
                let sample = match v["type"].as_str() {
                    Some("number") => json!(20),
                    Some("integer") => json!(1),
                    Some("boolean") => json!(true),
                    Some("array") => json!([[0, 0], [40, 0]]),
                    _ => match (i.id, k.as_str()) {
                        ("analysis.run", _) => json!("op"),
                        ("file.open_sample", _) => json!("astable"),
                        ("sim.control", "state") => json!("closed"),
                        _ => json!("x"),
                    },
                };
                args.insert(k.clone(), sample);
            }
            commands::from_id(i.id, &Value::Object(args))
                .unwrap_or_else(|e| panic!("{}: {e}", i.id));
            assert_eq!(command_id(&tool_name(i.id)), Some(i.id));
        }
    }

    #[test]
    fn dynamic_manual_sections_are_filled() {
        let all = ecs_mcp::static_sections()
            .into_iter()
            .chain(build_sections())
            .collect::<Vec<_>>();
        let find = |t: &str| {
            all.iter()
                .find(|s| s.topic == t)
                .unwrap_or_else(|| panic!("{t}"))
                .body
                .clone()
        };
        let commands = find("commands");
        assert!(
            commands.contains("`schematic_place`")
                && commands.contains("`document_describe`")
                && commands.contains("[read-only]")
        );
        let lib = find("library");
        assert!(
            lib.contains("`core.resistor`") && lib.contains("pins A, B") && lib.contains("R = 1k")
        );
        assert!(find("part-numbers").contains("2N2222A"));
        assert!(find("scripting-api").contains("```ts"));
    }

    #[test]
    fn server_end_to_end_over_the_protocol() {
        let (tx, _rx) = sync_channel(1);
        let mut server = Server::new(AppBackend::new(tx, egui::Context::default()));
        let reply = |s: &mut Server<AppBackend>, line: Value| -> Value {
            serde_json::from_str(&s.handle_line(&line.to_string()).unwrap()).unwrap()
        };
        let init = reply(
            &mut server,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "electrocraft");
        let list = reply(
            &mut server,
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        );
        assert!(list["result"]["tools"].as_array().unwrap().len() > 50);
        let manual = reply(
            &mut server,
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "get_manual", "arguments": {"topic": "library"}}}),
        );
        assert!(
            manual["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("core.resistor")
        );
        let res = reply(
            &mut server,
            json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": "manual://commands"}}),
        );
        assert!(
            res["result"]["contents"][0]["text"]
                .as_str()
                .unwrap()
                .contains("schematic_wire")
        );
    }
}
