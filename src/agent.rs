//! Machine-readable interface for AI agents.
//!
//! Two entry points share this module:
//!
//! * `amproute-studio --cli --json` — one JSON document on stdout, for shells
//!   and scripts (see [`crate::pipeline::PipelineResult`]);
//! * `amproute-mcp` — a JSON-RPC 2.0 / MCP server over stdio, for agents that
//!   can call tools. Every response is exactly one line of JSON; diagnostics
//!   go to stderr. The wire format is documented in `docs/agent-api.md`.
//!
//! The service keeps the last pipeline result in memory so an agent can fetch
//! the full report after a routing call without routing the board twice.

use crate::board::Board;
use crate::measure::{NetCurrent, NetExcitation};
use crate::pipeline::{PipelineConfig, PipelineResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Version of the wire contract. Bump when a field is removed or renamed.
pub const AGENT_API_VERSION: &str = "1.0";

#[derive(Debug, Clone, Deserialize)]
struct Request {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Clone, Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

/// In-process state of the MCP service.
#[derive(Default)]
pub struct AgentService {
    last: Option<PipelineResult>,
}

/// Description returned by `get_capabilities`, also useful for tests.
pub fn capabilities() -> Value {
    json!({
        "api_version": AGENT_API_VERSION,
        "product": "AmpRoute Studio",
        "layers_supported": ["F.Cu", "B.Cu"],
        "routing_layers_available_now": 2,
        "input_formats": ["kicad_pcb", "json", "built-in board id"],
        "output_files": ["board.gbr", "board_b.gbr", "board.svg", "board.pdf", "lut_print.pdf", "<board>.kicad_pcb"],
        "exit_codes": { "ok": 0, "invalid_board": 2, "routing_or_export_refused": 3 },
        "analysis": ["dc", "ac", "emi", "thermal"],
        "tools": ["get_capabilities", "list_boards", "list_extra_boards", "validate_board", "route_board", "get_result", "run_drc", "run_ac", "generate_netlist", "run_emi", "run_thermal", "get_quality_schema", "optimize_placement", "set_lut_profile", "suggest_jumpers"]
    })
}

fn tool_result(value: Value) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": value,
        "isError": false
    })
}

impl AgentService {
    /// Handle one JSON-RPC request line. Always returns a JSON object.
    pub fn handle_line(&mut self, line: &str) -> String {
        let parsed: Request = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                return serde_json::to_string(&Response {
                    jsonrpc: "2.0",
                    id: Value::Null,
                    result: None,
                    error: Some(RpcError {
                        code: -32700,
                        message: format!("parse error: {e}"),
                    }),
                })
                .expect("response serializes")
            }
        };
        let id = parsed.id.clone().unwrap_or(Value::Null);
        if parsed.jsonrpc != "2.0" {
            return self.error(id, -32600, "invalid Request: jsonrpc must be \"2.0\"");
        }
        let result = match self.dispatch(&parsed.method, &parsed.params) {
            Ok(value) => value,
            Err((code, message)) => return self.error(id, code, message),
        };
        serde_json::to_string(&Response {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        })
        .expect("response serializes")
    }

    fn error(&self, id: Value, code: i32, message: impl Into<String>) -> String {
        serde_json::to_string(&Response {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        })
        .expect("response serializes")
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, (i32, String)> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "amproute-mcp", "version": AGENT_API_VERSION }
            })),
            "tools/list" => Ok(json!({ "tools": [
                tool("get_capabilities", "Return API version, formats and supported layers"),
                tool("list_boards", "List the five built-in amplifier boards"),
                tool("list_extra_boards", "List ten agent-generated experimental amplifier boards"),
                tool("run_ac", "Run the linear AC proxy on a board with agent models"),
                tool("generate_netlist", "Return a ngspice deck for a board and models"),
                tool("run_emi", "Run geometry-based crosstalk and loop-area screens"),
                tool("run_thermal", "Run the transient junction-temperature proxy"),
                tool("get_quality_schema", "Return the quality JSON schema version and fields"),
                tool("optimize_placement", "Report placements for a board (opt-in, never applied implicitly)"),
                tool("set_lut_profile", "Validate an agent-supplied LUT profile"),
                tool("suggest_jumpers", "List nets that need a jumper on a single-layer board"),
                tool("validate_board", "Validate a .kicad_pcb or board JSON without routing"),
                tool("route_board", "Route a built-in board or a file and write all exports"),
                tool("get_result", "Return the last complete pipeline result"),
                tool("run_drc", "Run the own audit on a board file")
            ]})),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((-32602, "tools/call requires `name`".to_string()))?;
                let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let value = self.call_tool(name, &args)?;
                Ok(tool_result(value))
            }
            // Direct methods: same tools, raw result, for simple JSON-RPC clients.
            "get_capabilities" => Ok(capabilities()),
            "list_boards" => Ok(self.list_boards()),
            "list_extra_boards" => Ok(self.list_extra_boards()),
            "validate_board" => self.validate_board(params),
            "route_board" => self.route_board(params),
            "get_result" => self.get_result(),
            "run_drc" => self.run_drc(params),
            "run_ac" => self.run_ac(params),
            "generate_netlist" => self.generate_netlist(params),
            "run_emi" => self.run_emi(params),
            "run_thermal" => self.run_thermal(params),
            "get_quality_schema" => Ok(self.quality_schema()),
            "optimize_placement" => self.optimize_placement(params),
            "set_lut_profile" => self.set_lut_profile(params),
            "suggest_jumpers" => self.suggest_jumpers(params),
            other => Err((-32601, format!("method not found: {other}"))),
        }
    }

    fn call_tool(&mut self, name: &str, args: &Value) -> Result<Value, (i32, String)> {
        match name {
            "get_capabilities" => Ok(capabilities()),
            "list_boards" => Ok(self.list_boards()),
            "list_extra_boards" => Ok(self.list_extra_boards()),
            "validate_board" => self.validate_board(args),
            "route_board" => self.route_board(args),
            "get_result" => self.get_result(),
            "run_drc" => self.run_drc(args),
            "run_ac" => self.run_ac(args),
            "generate_netlist" => self.generate_netlist(args),
            "run_emi" => self.run_emi(args),
            "run_thermal" => self.run_thermal(args),
            "get_quality_schema" => Ok(self.quality_schema()),
            "optimize_placement" => self.optimize_placement(args),
            "set_lut_profile" => self.set_lut_profile(args),
            "suggest_jumpers" => self.suggest_jumpers(args),
            other => Err((-32602, format!("unknown tool: {other}"))),
        }
    }

    fn list_boards(&self) -> Value {
        let boards: Vec<Value> = crate::boards::BOARD_SPECS
            .iter()
            .map(|spec| {
                let board = (spec.build)();
                json!({
                    "id": spec.id,
                    "summary": spec.summary,
                    "width_mm": board.width_mm,
                    "height_mm": board.height_mm,
                    "nets": board.nets.len(),
                    "footprints": board.footprints.len()
                })
            })
            .collect();
        json!({ "boards": boards, "count": boards.len() })
    }

    fn list_extra_boards(&self) -> Value {
        let boards: Vec<Value> = crate::extra_boards::EXTRA_BOARD_SPECS
            .iter()
            .map(|spec| {
                let board = (spec.build)();
                json!({
                    "id": spec.id,
                    "summary": spec.summary,
                    "width_mm": board.width_mm,
                    "height_mm": board.height_mm,
                    "nets": board.nets.len(),
                    "footprints": board.footprints.len()
                })
            })
            .collect();
        json!({ "boards": boards, "count": boards.len() })
    }

    fn load_board(args: &Value) -> Result<Board, (i32, String)> {
        if let Some(id) = args.get("board").and_then(Value::as_str) {
            return crate::build_any(id)
                .ok_or_else(|| (-32602, format!("unknown board id: {id}")));
        }
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or((-32602, "provide `board` (built-in id) or `path` (board file)".to_string()))?;
        let data = std::fs::read_to_string(path)
            .map_err(|e| (-32602, format!("cannot read {path}: {e}")))?;
        if data.trim_start().starts_with('{') {
            Board::from_json(&data).map_err(|e| (-32602, format!("invalid board JSON: {e}")))
        } else {
            let stem = std::path::Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Imported board".into());
            crate::import::import_kicad_pcb_str_named(&data, &stem)
                .map(|r| r.board)
                .map_err(|e| (-32602, format!("cannot import {path}: {e}")))
        }
    }

    fn config(args: &Value) -> PipelineConfig {
        PipelineConfig {
            grid_mm: args.get("grid_mm").and_then(Value::as_f64).unwrap_or(0.5),
            time_budget_s: args.get("time_budget_s").and_then(Value::as_f64).unwrap_or(120.0),
            run_kicad_cli_drc: args
                .get("run_kicad_cli_drc")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            ..Default::default()
        }
    }

    fn validate_board(&self, args: &Value) -> Result<Value, (i32, String)> {
        let board = Self::load_board(args)?;
        let issues = board.validate();
        Ok(json!({
            "board": board.name,
            "valid": issues.is_empty(),
            "issues": issues,
            "width_mm": board.width_mm,
            "height_mm": board.height_mm,
            "nets": board.nets.len(),
            "pads": board.all_pads().len()
        }))
    }

    fn route_board(&mut self, args: &Value) -> Result<Value, (i32, String)> {
        let mut board = Self::load_board(args)?;
        if args.get("two_layer").and_then(Value::as_bool).unwrap_or(false) {
            board.two_layer = true;
        }
        let mut config = Self::config(args);
        if let Some(policy) = args.get("analysis_policy") {
            config.analysis_policy = serde_json::from_value(policy.clone())
                .map_err(|e| (-32602, format!("invalid analysis_policy: {e}")))?;
        }
        config.include_transient = args
            .get("transient")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(emi) = args.get("emi") {
            config.emi = Some(
                serde_json::from_value(emi.clone())
                    .map_err(|e| (-32602, format!("invalid emi config: {e}")))?,
            );
        }
        if args.get("ac_models").is_some() {
            config.ac = Some(Self::ac_config(args)?);
            config.prefer_ngspice = args
                .get("prefer_ngspice")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        }
        let excitation_arg = args
            .get("excitations")
            .or_else(|| args.get("measure_currents"));
        if let Some(list) = excitation_arg {
            if let Ok(excitations) = serde_json::from_value::<Vec<NetExcitation>>(list.clone()) {
                config.excitations = Some(excitations);
            } else {
                let currents: Vec<NetCurrent> = serde_json::from_value(list.clone())
                    .map_err(|e| (-32602, format!("invalid currents: {e}")))?;
                config.measure_currents = Some(currents);
            }
        }
        let out_dir = args
            .get("output_dir")
            .and_then(Value::as_str)
            .unwrap_or("output")
            .to_string();
        let result = crate::pipeline::run_pipeline(board, config, &out_dir)
            .map_err(|e| (-32603, format!("pipeline failed: {e}")))?;
        let summary = json!({
            "ok": result.ok,
            "board": result.board_name,
            "nets": result.nets,
            "routed_nets": result.routing.result.routed_nets,
            "failed_nets": result.routing.result.failed_nets,
            "copper_mm": result.final_copper.total_length_mm,
            "drc_violations": result.drc.violations,
            "quality": result.quality,
            "ac": result.ac,
            "emi": result.emi,
            "transient": result.transient,
            "exported_files": result.exported_files,
            "kicad_board_path": result.kicad_board_path,
            "elapsed_ms": result.elapsed_ms
        });
        self.last = Some(result);
        Ok(summary)
    }

    fn get_result(&self) -> Result<Value, (i32, String)> {
        match &self.last {
            Some(result) => serde_json::to_value(result)
                .map_err(|e| (-32603, format!("cannot serialize result: {e}"))),
            None => Err((-32001, "no route has been run in this session".to_string())),
        }
    }

    fn ac_config(args: &Value) -> Result<crate::ac::AcConfig, (i32, String)> {
        let mut config = crate::ac::AcConfig::default();
        if let Some(models) = args.get("ac_models") {
            config.models = serde_json::from_value(models.clone())
                .map_err(|e| (-32602, format!("invalid ac_models: {e}")))?;
        }
        if let Some(v) = args.get("ac_input_net").and_then(Value::as_str) {
            config.input_net = Some(v.into());
        }
        if let Some(v) = args.get("ac_output_net").and_then(Value::as_str) {
            config.output_net = Some(v.into());
        }
        Ok(config)
    }

    fn run_ac(&self, args: &Value) -> Result<Value, (i32, String)> {
        let board = Self::load_board(args)?;
        let config = Self::ac_config(args)?;
        let report = crate::ac::analyze(&board, &config);
        serde_json::to_value(report)
            .map_err(|e| (-32603, format!("cannot serialize AC report: {e}")))
    }

    fn generate_netlist(&self, args: &Value) -> Result<Value, (i32, String)> {
        let board = Self::load_board(args)?;
        let config = Self::ac_config(args)?;
        Ok(json!({ "netlist": crate::spice::generate_netlist(&board, &config) }))
    }

    fn run_emi(&self, args: &Value) -> Result<Value, (i32, String)> {
        let board = Self::load_board(args)?;
        let emi: crate::emi::EmiConfig = match args.get("emi") {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|e| (-32602, format!("invalid emi config: {e}")))?,
            None => crate::emi::EmiConfig::default(),
        };
        let tmp = std::env::temp_dir().join(format!("amproute_mcp_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).map_err(|e| (-32603, e.to_string()))?;
        let result = crate::pipeline::run_pipeline(
            board,
            PipelineConfig {
                grid_mm: 0.5,
                time_budget_s: 120.0,
                emi: Some(emi),
                ..Default::default()
            },
            tmp.to_str().unwrap(),
        )
        .map_err(|e| (-32603, format!("pipeline failed: {e}")))?;
        serde_json::to_value(result.emi.unwrap_or_default())
            .map_err(|e| (-32603, format!("cannot serialize EMI report: {e}")))
    }

    fn quality_schema(&self) -> Value {
        json!({
            "schema_version": crate::pipeline::QUALITY_SCHEMA_VERSION,
            "fields": [
                "assumed_currents", "electrical_ok", "thermal_ok", "ac_ok", "emi_ok",
                "violations", "schema_version", "model", "confidence", "assumptions", "summary"
            ],
            "blocking_policy": ["electrical", "thermal", "ac", "emi"],
            "note": "empty analysis_policy means every supplied analysis blocks ok"
        })
    }

    fn optimize_placement(&self, args: &Value) -> Result<Value, (i32, String)> {
        let board = Self::load_board(args)?;
        // Non-destructive report: the caller decides whether to apply changes.
        let suggestions: Vec<Value> = board
            .footprints
            .iter()
            .map(|fp| {
                json!({
                    "refdes": fp.refdes,
                    "x_mm": fp.x,
                    "y_mm": fp.y,
                    "rotation_deg": fp.rotation_deg,
                    "candidates_deg": [0, 180],
                    "reason": "THT part: 0/180 keeps pads on the same grid"
                })
            })
            .collect();
        Ok(json!({
            "board": board.name,
            "applied": false,
            "suggestions": suggestions,
            "validation": board.validate()
        }))
    }

    fn set_lut_profile(&self, args: &Value) -> Result<Value, (i32, String)> {
        let min_track = args.get("min_track_mm").and_then(Value::as_f64).unwrap_or(0.4);
        let min_clearance = args.get("min_clearance_mm").and_then(Value::as_f64).unwrap_or(0.3);
        let mut rules = crate::rules::LutRules::reliable();
        rules.min_track_mm = min_track;
        rules.min_clearance_mm = min_clearance;
        let report = rules.validate_profile();
        Ok(json!({
            "accepted": report.is_empty(),
            "profile": { "min_track_mm": rules.min_track_mm, "min_clearance_mm": rules.min_clearance_mm },
            "violations": report.violations.iter().map(|v| v.to_string()).collect::<Vec<_>>()
        }))
    }

    fn suggest_jumpers(&self, args: &Value) -> Result<Value, (i32, String)> {
        let board = Self::load_board(args)?;
        let tmp = std::env::temp_dir().join(format!("amproute_jump_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).map_err(|e| (-32603, e.to_string()))?;
        let result = crate::pipeline::run_pipeline(
            board.clone(),
            PipelineConfig {
                grid_mm: 0.5,
                time_budget_s: 120.0,
                ..Default::default()
            },
            tmp.to_str().unwrap(),
        )
        .map_err(|e| (-32603, format!("pipeline failed: {e}")))?;
        let suggestions: Vec<Value> = result
            .routing
            .result
            .failed_nets
            .iter()
            .map(|failed| {
                let name = failed.split(':').next().unwrap_or(failed).trim();
                let net = board.nets.iter().find(|n| n.name == name);
                json!({
                    "net": name,
                    "terminals": net.map(|n| board.net_terminals(n)).unwrap_or_default(),
                    "reason": failed
                })
            })
            .collect();
        Ok(json!({ "jumpers": suggestions, "count": suggestions.len() }))
    }

    fn run_thermal(&self, args: &Value) -> Result<Value, (i32, String)> {
        let board = Self::load_board(args)?;
        let tmp = std::env::temp_dir().join(format!("amproute_mcp_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).map_err(|e| (-32603, e.to_string()))?;
        let result = crate::pipeline::run_pipeline(
            board,
            PipelineConfig {
                grid_mm: 0.5,
                time_budget_s: 120.0,
                include_transient: true,
                ..Default::default()
            },
            tmp.to_str().unwrap(),
        )
        .map_err(|e| (-32603, format!("pipeline failed: {e}")))?;
        serde_json::to_value(result.transient.unwrap_or_default())
            .map_err(|e| (-32603, format!("cannot serialize thermal report: {e}")))
    }

    fn run_drc(&self, args: &Value) -> Result<Value, (i32, String)> {
        let board = Self::load_board(args)?;
        let report = crate::drc::audit(&board, None, None);
        serde_json::to_value(&report)
            .map_err(|e| (-32603, format!("cannot serialize DRC report: {e}")))
    }
}

fn tool(name: &str, description: &str) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": {} }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(service: &mut AgentService, method: &str, id: u32) -> Value {
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\",\"params\":{{}}}}"
        );
        let response: Value = serde_json::from_str(&service.handle_line(&line)).unwrap();
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], id);
        response["result"].clone()
    }

    #[test]
    fn initialize_and_tools_list_work() {
        let mut service = AgentService::default();
        let init = call(&mut service, "initialize", 1);
        assert_eq!(init["serverInfo"]["name"], "amproute-mcp");
        let tools = call(&mut service, "tools/list", 2);
        assert!(tools["tools"].as_array().unwrap().len() >= 6);
    }

    #[test]
    fn list_boards_reports_five_boards() {
        let mut service = AgentService::default();
        let boards = call(&mut service, "list_boards", 3);
        assert_eq!(boards["count"], 5);
    }

    #[test]
    fn list_extra_boards_offers_ten_agent_designs() {
        let mut service = AgentService::default();
        let boards = call(&mut service, "list_extra_boards", 9);
        assert_eq!(boards["count"], 10);
    }

    #[test]
    fn tools_call_returns_structured_content() {
        let mut service = AgentService::default();
        let line = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_capabilities","arguments":{}}}"#;
        let response: Value = serde_json::from_str(&service.handle_line(line)).unwrap();
        assert_eq!(response["result"]["structuredContent"]["api_version"], AGENT_API_VERSION);
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn route_and_get_result_share_the_same_run() {
        let mut service = AgentService::default();
        let tmp = tempfile::tempdir().unwrap();
        let line = json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "route_board",
            "params": {
                "board": "lm386-mini",
                "output_dir": tmp.path().to_string_lossy(),
                "time_budget_s": 20.0
            }
        })
        .to_string();
        let response: Value = serde_json::from_str(&service.handle_line(&line)).unwrap();
        assert!(response["error"].is_null(), "{response}");
        assert_eq!(response["result"]["board"], "LM386 Mini");
        let full = call(&mut service, "get_result", 6);
        assert_eq!(full["board_name"], "LM386 Mini");
        assert!(!full["final_copper"]["segments"].as_array().unwrap().is_empty());
    }

    #[test]
    fn agent_can_supply_real_currents_for_a_quality_gate() {
        let mut service = AgentService::default();
        let tmp = tempfile::tempdir().unwrap();
        let line = json!({
            "jsonrpc": "2.0", "id": 10, "method": "route_board",
            "params": {
                "board": "lm386-mini",
                "output_dir": tmp.path().to_string_lossy(),
                "time_budget_s": 20.0,
                "measure_currents": [{"net": "V+", "current_a": 0.1, "peak_a": 0.2}]
            }
        })
        .to_string();
        let response: Value = serde_json::from_str(&service.handle_line(&line)).unwrap();
        assert!(response["error"].is_null(), "{response}");
        assert_eq!(response["result"]["quality"]["assumed_currents"], false);
    }

    #[test]
    fn run_ac_returns_a_proxy_report() {
        let mut service = AgentService::default();
        let line = r#"{"jsonrpc":"2.0","id":11,"method":"run_ac","params":{"board":"lm386-mini","ac_models":{}}}"#;
        let response: Value = serde_json::from_str(&service.handle_line(line)).unwrap();
        assert!(response["error"].is_null(), "{response}");
        assert_eq!(response["result"]["model"], "proxy");
        assert!(response["result"]["points"].as_array().unwrap().len() > 2);
    }

    #[test]
    fn run_emi_reports_proxy_model() {
        let mut service = AgentService::default();
        let line = r#"{"jsonrpc":"2.0","id":12,"method":"run_emi","params":{"board":"lm386-mini"}}"#;
        let response: Value = serde_json::from_str(&service.handle_line(line)).unwrap();
        assert!(response["error"].is_null(), "{response}");
        assert_eq!(response["result"]["model"], "proxy");
    }

    #[test]
    fn new_agent_tools_are_wired() {
        let mut service = AgentService::default();
        let schema = call(&mut service, "get_quality_schema", 20);
        assert_eq!(schema["schema_version"], "1.0");
        let profile = r#"{"jsonrpc":"2.0","id":21,"method":"set_lut_profile","params":{"min_track_mm":0.4,"min_clearance_mm":0.3}}"#;
        let response: Value = serde_json::from_str(&service.handle_line(profile)).unwrap();
        assert_eq!(response["result"]["accepted"], true);
        let placement = r#"{"jsonrpc":"2.0","id":22,"method":"optimize_placement","params":{"board":"lm386-mini"}}"#;
        let response: Value = serde_json::from_str(&service.handle_line(placement)).unwrap();
        assert_eq!(response["result"]["applied"], false);
    }

    #[test]
    fn errors_are_json_rpc_errors() {
        let mut service = AgentService::default();
        let bad = r#"{"jsonrpc":"2.0","id":7,"method":"no_such_method","params":{}}"#;
        let response: Value = serde_json::from_str(&service.handle_line(bad)).unwrap();
        assert_eq!(response["error"]["code"], -32601);
        let not_json = service.handle_line("not json");
        let response: Value = serde_json::from_str(&not_json).unwrap();
        assert_eq!(response["error"]["code"], -32700);
    }

    #[test]
    fn validate_rejects_unknown_board() {
        let mut service = AgentService::default();
        let line = r#"{"jsonrpc":"2.0","id":8,"method":"validate_board","params":{"board":"nope"}}"#;
        let response: Value = serde_json::from_str(&service.handle_line(line)).unwrap();
        assert_eq!(response["error"]["code"], -32602);
    }
}
