//! End-to-end check of the stdio MCP server: the agent launches the binary,
//! writes JSON-RPC requests and reads one JSON response per line.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn server() -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_amproute-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("amproute-mcp starts")
}

fn ask(server: &mut std::process::Child, request: Value) -> Value {
    let stdin = server.stdin.as_mut().unwrap();
    writeln!(stdin, "{request}").unwrap();
    stdin.flush().unwrap();
    let mut reader = BufReader::new(server.stdout.as_mut().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    // A BufReader is recreated per call; buffered extra lines would be lost, so
    // callers only send one request at a time.
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad response `{line}`: {e}"))
}

#[test]
fn mcp_server_answers_initialize_and_list_boards() {
    let mut server = server();
    let init = ask(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "amproute-mcp");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");

    let boards = ask(
        &mut server,
        json!({"jsonrpc": "2.0", "id": 2, "method": "list_boards", "params": {}}),
    );
    assert_eq!(boards["result"]["count"], 5);
    server.kill().unwrap();
}

#[test]
fn mcp_server_routes_a_builtin_board_and_reports_files() {
    let tmp = tempfile::tempdir().unwrap();
    let mut server = server();
    let response = ask(
        &mut server,
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "route_board",
            "params": {
                "board": "lm386-mini",
                "output_dir": tmp.path().to_string_lossy(),
                "time_budget_s": 20.0
            }
        }),
    );
    assert!(response["error"].is_null(), "{response}");
    assert_eq!(response["result"]["board"], "LM386 Mini");
    let files = response["result"]["exported_files"].as_array().unwrap();
    assert!(files.len() >= 4, "expected the export set, got {files:?}");
    assert!(std::path::Path::new(&format!("{}/lut_print.pdf", tmp.path().to_string_lossy())).exists());
    server.kill().unwrap();
}
