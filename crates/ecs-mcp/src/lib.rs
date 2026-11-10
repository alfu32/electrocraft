//! Model Context Protocol server for Electrocraft (D14: every action is a
//! command with a stable id; this crate maps them to MCP tools).
//!
//! The crate knows the protocol and the manual, nothing about the app: the
//! app supplies a [`Backend`] (its tools, how to run one on the UI thread,
//! and the manual sections that depend on the running build). Transport is
//! newline-delimited JSON-RPC on stdio, so any MCP client can launch
//! `electrocraft --mcp`.
//!
//! How a long manual reaches the model, in order of reliability:
//!
//! 1. **Server instructions** (sent at `initialize`): a short orientation
//!    and a pointer to `get_manual`.
//! 2. **Tool descriptions**: each ends with "For details, call get_manual
//!    with topic …".
//! 3. **`get_manual` tool**: a table of contents, or one section (or a text
//!    search), so the model loads only what it needs. Tools work in every client.
//! 4. **Resources** `manual://index` and `manual://<topic>`, for clients
//!    that list and attach resources (many agents read them only when the
//!    user attaches them, so this complements the tool).

use serde_json::{Value, json};
use std::io::{BufRead, Write};

/// Protocol versions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Name of the manual tool.
pub const MANUAL_TOOL: &str = "get_manual";

/// Orientation sent in the `initialize` result.
pub const INSTRUCTIONS: &str = "\
Electrocraft is a circuit simulator with a live schematic editor; you drive the \
editor the user has open. Build circuits with tools (place parts, wire them, set \
values), run the simulation, and read results back.

Start by calling get_manual with no topic: it lists the manual's topics. Read \
`workflow` before building anything and `schematic` for coordinates (a 20-unit \
grid; wires connect only at pins, ends and junctions). Every tool description \
names the topic with its details: call get_manual with that topic rather than \
guessing arguments. The same text is available as resources (manual://index).

Use document_describe to see what is on the canvas (parts, pin positions and \
nets, problems) and sim_read for live voltages and currents. Edits are undoable \
by the user (edit_undo). Online part catalogs are off unless the user enabled \
them in Settings; you cannot enable them.";

/// One tool as listed to the client.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub title: String,
    pub description: String,
    /// JSON Schema of the arguments (an object schema).
    pub input_schema: Value,
    /// The tool only reads state.
    pub read_only: bool,
}

/// One manual section.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// Short id used by `get_manual` and `manual://<topic>`.
    pub topic: String,
    pub title: String,
    /// One line for the table of contents.
    pub summary: String,
    /// Markdown.
    pub body: String,
}

impl Section {
    pub fn new(topic: &str, title: &str, summary: &str, body: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            title: title.into(),
            summary: summary.into(),
            body: body.into(),
        }
    }
}

/// What the app provides.
pub trait Backend {
    /// Application name and version for `serverInfo`.
    fn info(&self) -> (String, String);
    /// Every tool except `get_manual` (the server adds that one).
    fn tools(&self) -> Vec<ToolDef>;
    /// Run a tool. `Ok` is the text shown to the model, `Err` a tool error
    /// (reported with `isError`, not as a protocol error).
    fn call(&mut self, tool: &str, args: &Value) -> Result<String, String>;
    /// Manual sections that depend on the running build (command list,
    /// component library, scripting API); they extend or replace static ones.
    fn sections(&self) -> Vec<Section> {
        Vec::new()
    }
}

/// The manual sections that do not depend on the build, written in
/// `docs/manual/*.md` (so people can read them too).
pub fn static_sections() -> Vec<Section> {
    vec![
        Section::new(
            "overview",
            "Overview",
            "What Electrocraft is, how the tools map to the app, limits",
            include_str!("../../../docs/manual/overview.md"),
        ),
        Section::new(
            "workflow",
            "Workflow: building and checking a circuit",
            "Step by step recipe, with a worked example",
            include_str!("../../../docs/manual/workflow.md"),
        ),
        Section::new(
            "schematic",
            "Schematic editing",
            "Coordinates, grid, rotation, pins, wires, net labels, reference designators",
            include_str!("../../../docs/manual/schematic.md"),
        ),
        Section::new(
            "parameters",
            "Values and parameters",
            "Engineering notation, part numbers, how a value is set",
            include_str!("../../../docs/manual/parameters.md"),
        ),
        Section::new(
            "simulation",
            "Simulation and live values",
            "Run, pause, step, reset, controls, reading results",
            include_str!("../../../docs/manual/simulation.md"),
        ),
        Section::new(
            "scope",
            "Oscilloscope and formulas",
            "Channels as formulas over codes (out, R1.I, Q1.C.V)",
            include_str!("../../../docs/manual/scope.md"),
        ),
        Section::new(
            "analyses",
            "Analyses",
            "DC operating point, DC sweep, transient, AC",
            include_str!("../../../docs/manual/analyses.md"),
        ),
        Section::new(
            "components",
            "Your own components",
            "Symbols, ports, nested schematics, JavaScript implementations",
            include_str!("../../../docs/manual/components.md"),
        ),
        Section::new(
            "microcontrollers",
            "Microcontrollers",
            "PIC12/16/24/32 chips, assembly programs, set_program",
            include_str!("../../../docs/manual/microcontrollers.md"),
        ),
        Section::new(
            "catalogs",
            "Online part catalogs and sourcing",
            "Searching JLCPCB, DigiKey, Mouser, TME, Farnell; sourcing; BOM; consent",
            include_str!("../../../docs/manual/catalogs.md"),
        ),
        Section::new(
            "files",
            "Files and formats",
            "The .ecs schematic file, opening and saving",
            include_str!("../../../docs/manual/files.md"),
        ),
    ]
}

/// Merge the backend's sections over the static ones, keeping order.
pub fn sections(backend: &dyn Backend) -> Vec<Section> {
    let mut all = static_sections();
    for s in backend.sections() {
        match all.iter_mut().find(|x| x.topic == s.topic) {
            Some(slot) => *slot = s,
            None => all.push(s),
        }
    }
    all
}

/// The table of contents as markdown.
pub fn contents(all: &[Section]) -> String {
    let mut out = String::from(
        "# Electrocraft manual\n\nCall get_manual with `topic` set to one of these (or `query` to search the manual). The same sections are resources: `manual://<topic>`.\n\n",
    );
    for s in all {
        out += &format!("- `{}`: {}. {}\n", s.topic, s.title, s.summary);
    }
    out
}

/// What `get_manual` answers: Ok is the text, Err is an error text (unknown topic).
pub fn manual(all: &[Section], topic: Option<&str>, query: Option<&str>) -> Result<String, String> {
    let topic = topic.map(str::trim).filter(|t| !t.is_empty());
    let query = query.map(str::trim).filter(|t| !t.is_empty());
    if let Some(t) = topic {
        let want = t.trim_start_matches("manual://").to_lowercase();
        if want == "index" || want == "contents" {
            return Ok(contents(all));
        }
        return match all.iter().find(|s| s.topic == want) {
            Some(s) => Ok(format!("# {}\n\n{}", s.title, s.body)),
            None => Err(format!("Unknown topic `{t}`.\n\n{}", contents(all))),
        };
    }
    if let Some(q) = query {
        let words: Vec<String> = q
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let mut out = String::new();
        let mut hits = 0;
        for s in all {
            for (k, line) in s.body.lines().enumerate() {
                let l = line.to_lowercase();
                if words.iter().all(|w| l.contains(w)) {
                    hits += 1;
                    if hits <= 40 {
                        out += &format!("- `{}` line {}: {}\n", s.topic, k + 1, line.trim());
                    }
                }
            }
        }
        return Ok(if hits == 0 {
            format!("Nothing in the manual matches `{q}`.\n\n{}", contents(all))
        } else {
            format!(
                "{hits} matching line(s){}:\n\n{out}\nCall get_manual with the topic to read a section.",
                if hits > 40 { " (first 40 shown)" } else { "" }
            )
        });
    }
    Ok(contents(all))
}

fn manual_tool() -> ToolDef {
    ToolDef {
        name: MANUAL_TOOL.into(),
        title: "Electrocraft manual".into(),
        description: "The Electrocraft manual. Call with no arguments for the table of contents, with `topic` for one section (start with `workflow`, then `schematic`), or with `query` to search it. Other tools say which topic holds their details: read it instead of guessing arguments.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "topic": {"type": "string", "description": "Section id from the table of contents, e.g. workflow, schematic, scope, commands"},
                "query": {"type": "string", "description": "Words to search for in the manual (all must appear on a line)"}
            }
        }),
        read_only: true,
    }
}

/// The MCP server over a backend.
pub struct Server<B: Backend> {
    backend: B,
    initialized: bool,
}

fn ok(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn err(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

impl<B: Backend> Server<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            initialized: false,
        }
    }

    pub fn backend(&mut self) -> &mut B {
        &mut self.backend
    }

    fn tool_list(&self) -> Vec<Value> {
        let mut tools = vec![manual_tool()];
        tools.extend(self.backend.tools());
        tools
            .into_iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "title": t.title,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                    "annotations": {"title": t.title, "readOnlyHint": t.read_only, "openWorldHint": false},
                })
            })
            .collect()
    }

    fn resource_list(&self) -> Vec<Value> {
        let all = sections(&self.backend);
        let mut out = vec![
            json!({"uri": "manual://index", "name": "index", "title": "Electrocraft manual: contents", "description": "Table of contents of the manual", "mimeType": "text/markdown"}),
        ];
        for s in &all {
            out.push(json!({"uri": format!("manual://{}", s.topic), "name": s.topic, "title": s.title, "description": s.summary, "mimeType": "text/markdown"}));
        }
        out
    }

    /// Handle one JSON-RPC message; None for notifications.
    fn handle(&mut self, msg: &Value) -> Option<Value> {
        let method = msg["method"].as_str().unwrap_or("");
        let id = msg.get("id").cloned();
        // Responses to requests we never send, and notifications.
        let Some(id) = id else {
            if method == "notifications/initialized" {
                self.initialized = true;
            }
            return None;
        };
        if method.is_empty() {
            return None;
        }
        let params = &msg["params"];
        Some(match method {
            "initialize" => {
                let want = params["protocolVersion"].as_str().unwrap_or("");
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|v| **v == want)
                    .unwrap_or(&PROTOCOL_VERSIONS[1]);
                let (name, v) = self.backend.info();
                ok(
                    &id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": {"tools": {"listChanged": false}, "resources": {"subscribe": false, "listChanged": false}},
                        "serverInfo": {"name": name, "title": "Electrocraft circuit simulator", "version": v},
                        "instructions": INSTRUCTIONS,
                    }),
                )
            }
            "ping" => ok(&id, json!({})),
            "tools/list" => ok(&id, json!({"tools": self.tool_list()})),
            "tools/call" => {
                let Some(name) = params["name"].as_str() else {
                    return Some(err(&id, -32602, "tools/call needs a tool name"));
                };
                let args = if params["arguments"].is_null() {
                    json!({})
                } else {
                    params["arguments"].clone()
                };
                if name == MANUAL_TOOL {
                    let all = sections(&self.backend);
                    return Some(ok(
                        &id,
                        match manual(&all, args["topic"].as_str(), args["query"].as_str()) {
                            Ok(t) => text_result(t, false),
                            Err(t) => text_result(t, true),
                        },
                    ));
                }
                if !self.backend.tools().iter().any(|t| t.name == name) {
                    return Some(err(&id, -32602, &format!("Unknown tool: {name}")));
                }
                ok(
                    &id,
                    match self.backend.call(name, &args) {
                        Ok(t) => text_result(t, false),
                        Err(t) => text_result(t, true),
                    },
                )
            }
            "resources/list" => ok(&id, json!({"resources": self.resource_list()})),
            "resources/templates/list" => ok(
                &id,
                json!({"resourceTemplates": [{"uriTemplate": "manual://{topic}", "name": "manual-section", "title": "Manual section", "description": "One section of the Electrocraft manual (see manual://index for topics)", "mimeType": "text/markdown"}]}),
            ),
            "resources/read" => {
                let Some(uri) = params["uri"].as_str() else {
                    return Some(err(&id, -32602, "resources/read needs a uri"));
                };
                let Some(topic) = uri.strip_prefix("manual://") else {
                    return Some(err(&id, -32002, &format!("Resource not found: {uri}")));
                };
                let all = sections(&self.backend);
                match manual(&all, Some(topic), None) {
                    Ok(text) => ok(
                        &id,
                        json!({"contents": [{"uri": uri, "mimeType": "text/markdown", "text": text}]}),
                    ),
                    Err(_) => err(&id, -32002, &format!("Resource not found: {uri}")),
                }
            }
            "prompts/list" => ok(&id, json!({"prompts": []})),
            _ => err(&id, -32601, &format!("Method not found: {method}")),
        })
    }

    /// Handle one line (a message or a batch); the reply line, if any.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return Some(err(&Value::Null, -32700, &format!("Parse error: {e}")).to_string());
            }
        };
        match msg {
            Value::Array(batch) => {
                let replies: Vec<Value> = batch.iter().filter_map(|m| self.handle(m)).collect();
                (!replies.is_empty()).then(|| Value::Array(replies).to_string())
            }
            other => self.handle(&other).map(|v| v.to_string()),
        }
    }

    /// Read lines until the input closes, writing replies.
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if let Some(reply) = self.handle_line(&line) {
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
        Ok(())
    }
}

/// JSON Schema for a registry command's argument description (the
/// `{"name": "what it holds"}` objects in the command catalog).
pub fn schema_from_args(args: &Value) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    if let Some(map) = args.as_object() {
        for (k, v) in map {
            let text = v.as_str().unwrap_or("");
            let lower = text.to_lowercase();
            let optional = lower.starts_with("optional");
            let kind = if text.starts_with('[') {
                json!({"type": "array", "items": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2}})
            } else if lower.contains("boolean") {
                json!({"type": "boolean"})
            } else if matches!(k.as_str(), "index" | "rot")
                || lower.contains("tab number")
                || lower.contains("integer")
                || text.starts_with("0..")
            {
                json!({"type": "integer"})
            } else if matches!(k.as_str(), "x" | "y")
                || lower.starts_with("number")
                || lower.starts_with("0.05 to")
            {
                json!({"type": "number"})
            } else {
                json!({"type": "string"})
            };
            let mut prop = kind;
            prop["description"] = json!(text);
            properties.insert(k.clone(), prop);
            if !optional {
                required.push(json!(k));
            }
        }
    }
    let mut schema =
        json!({"type": "object", "properties": properties, "additionalProperties": false});
    if !required.is_empty() {
        schema["required"] = Value::Array(required);
    }
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        calls: Vec<(String, Value)>,
    }

    impl Backend for Fake {
        fn info(&self) -> (String, String) {
            ("electrocraft".into(), "9.9.9".into())
        }
        fn tools(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "schematic_place".into(),
                title: "Place a part".into(),
                description: "Place a part. For details call get_manual with topic schematic."
                    .into(),
                input_schema: schema_from_args(
                    &json!({"def": "component id", "x": "number", "rot": "0..3", "note": "optional text"}),
                ),
                read_only: false,
            }]
        }
        fn call(&mut self, tool: &str, args: &Value) -> Result<String, String> {
            self.calls.push((tool.into(), args.clone()));
            if args["def"] == "bad" {
                Err("unknown component bad".into())
            } else {
                Ok("placed R1".into())
            }
        }
        fn sections(&self) -> Vec<Section> {
            vec![
                Section::new(
                    "commands",
                    "Commands",
                    "Every command",
                    "# list\n- schematic_place",
                ),
                Section::new(
                    "overview",
                    "Overview (live)",
                    "replaced",
                    "live overview text",
                ),
            ]
        }
    }

    fn server() -> Server<Fake> {
        Server::new(Fake { calls: Vec::new() })
    }

    fn rpc(s: &mut Server<Fake>, msg: Value) -> Value {
        serde_json::from_str(&s.handle_line(&msg.to_string()).expect("reply")).unwrap()
    }

    #[test]
    fn initialize_negotiates_and_carries_instructions() {
        let mut s = server();
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}),
        );
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "electrocraft");
        assert!(
            r["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("get_manual")
        );
        assert!(
            r["result"]["capabilities"]["tools"].is_object()
                && r["result"]["capabilities"]["resources"].is_object()
        );
        // An unknown version gets a supported one.
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}),
        );
        assert!(PROTOCOL_VERSIONS.contains(&r["result"]["protocolVersion"].as_str().unwrap()));
        // Notifications get no reply.
        assert!(
            s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none()
        );
    }

    #[test]
    fn tools_list_has_the_manual_tool_first_and_annotations() {
        let mut s = server();
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );
        let tools = r["result"]["tools"].as_array().unwrap();
        assert_eq!(tools[0]["name"], "get_manual");
        assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
        assert_eq!(tools[1]["name"], "schematic_place");
        assert!(
            tools[1]["description"]
                .as_str()
                .unwrap()
                .contains("get_manual")
        );
        let schema = &tools[1]["inputSchema"];
        assert_eq!(schema["properties"]["x"]["type"], "number");
        assert_eq!(schema["properties"]["rot"]["type"], "integer");
        assert_eq!(schema["required"], json!(["def", "rot", "x"]));
    }

    #[test]
    fn tool_calls_report_errors_in_band() {
        let mut s = server();
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "schematic_place", "arguments": {"def": "core.resistor", "x": 20}}}),
        );
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(r["result"]["content"][0]["text"], "placed R1");
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "schematic_place", "arguments": {"def": "bad"}}}),
        );
        assert_eq!(r["result"]["isError"], true);
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "nope"}}),
        );
        assert_eq!(r["error"]["code"], -32602);
        assert_eq!(s.backend().calls.len(), 2);
    }

    #[test]
    fn manual_tool_serves_contents_sections_and_search() {
        let mut s = server();
        let call = |s: &mut Server<Fake>, args: Value| {
            rpc(
                s,
                json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "get_manual", "arguments": args}}),
            )
        };
        let toc = call(&mut s, json!({}));
        let text = toc["result"]["content"][0]["text"].as_str().unwrap();
        for topic in [
            "overview",
            "workflow",
            "schematic",
            "scope",
            "catalogs",
            "commands",
        ] {
            assert!(
                text.contains(&format!("`{topic}`")),
                "{topic} missing from the contents"
            );
        }
        // The backend's section replaces the static one with the same topic.
        let r = call(&mut s, json!({"topic": "overview"}));
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("live overview text")
        );
        let r = call(&mut s, json!({"topic": "schematic"}));
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("grid")
        );
        let r = call(&mut s, json!({"topic": "nonsense"}));
        assert_eq!(r["result"]["isError"], true);
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("`workflow`")
        );
        let r = call(&mut s, json!({"query": "junction"}));
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("matching line")
        );
    }

    #[test]
    fn manual_resources_list_and_read() {
        let mut s = server();
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}),
        );
        let uris: Vec<&str> = r["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["uri"].as_str().unwrap())
            .collect();
        assert!(
            uris.contains(&"manual://index")
                && uris.contains(&"manual://workflow")
                && uris.contains(&"manual://commands")
        );
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 2, "method": "resources/read", "params": {"uri": "manual://scope"}}),
        );
        assert_eq!(r["result"]["contents"][0]["mimeType"], "text/markdown");
        assert!(
            r["result"]["contents"][0]["text"]
                .as_str()
                .unwrap()
                .contains("formula")
        );
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": "manual://index"}}),
        );
        assert!(
            r["result"]["contents"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Electrocraft manual")
        );
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": "file:///etc/passwd"}}),
        );
        assert_eq!(r["error"]["code"], -32002);
    }

    #[test]
    fn protocol_errors_and_batches() {
        let mut s = server();
        let r: Value = serde_json::from_str(&s.handle_line("{not json").unwrap()).unwrap();
        assert_eq!(r["error"]["code"], -32700);
        let r = rpc(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 1, "method": "nope/nope"}),
        );
        assert_eq!(r["error"]["code"], -32601);
        let r: Value = serde_json::from_str(&s.handle_line(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"}]"#).unwrap()).unwrap();
        assert_eq!(r.as_array().unwrap().len(), 1);
        let mut out = Vec::new();
        s.serve(
            "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n\n".as_bytes(),
            &mut out,
        )
        .unwrap();
        assert_eq!(String::from_utf8(out).unwrap().lines().count(), 1);
    }

    #[test]
    fn every_manual_section_is_substantial() {
        for s in static_sections() {
            assert!(s.body.len() > 400, "{} is too short", s.topic);
            assert!(!s.summary.is_empty() && !s.title.is_empty());
        }
    }
}
