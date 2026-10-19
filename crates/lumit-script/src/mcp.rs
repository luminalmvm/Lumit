//! MCP, the protocol other programs use to call Lumit's commands as tools.
//!
//! # In plain terms
//!
//! A program that speaks MCP sends one JSON-RPC message a line, and
//! [`answer`] turns each into the line to send back. The tools are the
//! commands in [`COMMANDS`], spelt `layer_add` for `layer.add` since some
//! clients take no dots in a tool name, plus `script_run`, which runs a
//! JavaScript script as one undo step.
//!
//! Both ways a client can start are answered: the `initialize` handshake of
//! the 2025 revisions, and the 2026 one where every request names its
//! version. Nothing here reads, writes or waits, beyond the frame a client
//! asks to see, so the command line's standard input and Lumit's own
//! endpoint can share it.

use serde_json::{json, Value};

use crate::reach::Reach;
use crate::{commands, run, Host, Limits, RunOptions, COMMANDS};

/// The revision where each request carries its version.
const MODERN: &str = "2026-07-28";
/// The revisions that start with `initialize`, newest first.
const HANDSHAKE: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
const CAPABILITIES_KEY: &str = "io.modelcontextprotocol/clientCapabilities";
/// How long a client may keep the tool list. It only changes with Lumit.
const KEEP_MS: u64 = 3_600_000;
/// The largest frame sent back as a picture. A bigger one is only named.
const MAX_PICTURE: u64 = 2 << 20;

const SCRIPT_TOOL: &str = "script_run";
const SCRIPT_SCHEMA: &str = r#"{"type":"object","properties":{"source":{"type":"string","description":"The script, as lumit-cli run takes it, with the lumit object and After Effects' objects"},"name":{"type":"string","description":"What its undo step is called"}},"required":["source"],"additionalProperties":false}"#;

/// A JSON-RPC error: its code, message and any data.
type Failure = (i64, String, Option<Value>);

/// The reply to one message, `line`, or `None` when it needs none, as a
/// notification does. The reply is one line with no line break in it.
pub fn answer(line: &str, host: &dyn Host) -> Option<String> {
    reply_to(line, Ok(host))
}

/// [`answer`] while there is no project to work on: a tool call answers
/// `why`, and everything else as usual.
pub fn answer_without(line: &str, why: &str) -> Option<String> {
    reply_to(line, Err(why))
}

fn reply_to(line: &str, host: Result<&dyn Host, &str>) -> Option<String> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(e) => {
            let why = format!("That is not JSON: {e}.");
            return Some(reply(Value::Null, Err((-32700, why, None))));
        }
    };
    let Some(object) = message.as_object() else {
        let why = "A message is one JSON object.".to_owned();
        return Some(reply(Value::Null, Err((-32600, why, None))));
    };
    // Without an id it is a notification, which is never answered, or a
    // client's reply to a request, which this never sends.
    let id = object.get("id")?.clone();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        if object.contains_key("result") || object.contains_key("error") {
            return None;
        }
        let why = "A request needs a method.".to_owned();
        return Some(reply(id, Err((-32600, why, None))));
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let meta = params.get("_meta");
    let version = meta
        .and_then(|m| m.get(VERSION_KEY))
        .and_then(Value::as_str);
    let result = match version {
        Some(version) if version != MODERN && !HANDSHAKE.contains(&version) => Err((
            -32022,
            "Unsupported protocol version".to_owned(),
            Some(json!({ "supported": supported(), "requested": version })),
        )),
        Some(_) if meta.and_then(|m| m.get(CAPABILITIES_KEY)).is_none() => Err((
            -32602,
            format!("The request's _meta needs {CAPABILITIES_KEY}."),
            None,
        )),
        _ => outcome(method, &params, host),
    };
    // A request naming its version gets the fields that revision asks of
    // every result.
    let result = result.map(|mut result| {
        if version.is_some() {
            result["resultType"] = json!("complete");
            result["_meta"] = json!({ "io.modelcontextprotocol/serverInfo": server_info() });
        }
        result
    });
    Some(reply(id, result))
}

fn reply(id: Value, result: Result<Value, Failure>) -> String {
    let message = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message, data)) => {
            let mut error = json!({ "code": code, "message": message });
            if let Some(data) = data {
                error["data"] = data;
            }
            json!({ "jsonrpc": "2.0", "id": id, "error": error })
        }
    };
    message.to_string()
}

fn outcome(method: &str, params: &Value, host: Result<&dyn Host, &str>) -> Result<Value, Failure> {
    let capabilities = json!({ "tools": {} });
    match method {
        "initialize" => {
            // The client's revision when it is one of ours, else our newest
            // and the client decides.
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked
                .and_then(|v| HANDSHAKE.iter().find(|h| **h == v))
                .unwrap_or(&HANDSHAKE[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": capabilities,
                "serverInfo": server_info(),
            }))
        }
        "server/discover" => Ok(json!({
            "supportedVersions": supported(),
            "capabilities": capabilities,
            "_meta": { "io.modelcontextprotocol/serverInfo": server_info() },
            "ttlMs": KEEP_MS,
            "cacheScope": "public",
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools(), "ttlMs": KEEP_MS, "cacheScope": "public" })),
        "tools/call" => call(params, host),
        _ => Err((-32601, format!("There is no method called {method}."), None)),
    }
}

fn supported() -> Vec<&'static str> {
    std::iter::once(MODERN)
        .chain(HANDSHAKE.iter().copied())
        .collect()
}

fn server_info() -> Value {
    json!({ "name": "lumit", "version": env!("CARGO_PKG_VERSION") })
}

/// `layer.add` as a tool name: letters, digits, `_` and `-` only.
pub(crate) fn tool_name(command: &str) -> String {
    command.replace('.', "_")
}

fn tools() -> Vec<Value> {
    let tool = |name: String, description: &str, schema: &str| {
        let schema: Value =
            serde_json::from_str(schema).unwrap_or_else(|_| json!({ "type": "object" }));
        json!({ "name": name, "description": description, "inputSchema": schema })
    };
    COMMANDS
        .iter()
        .map(|c| tool(tool_name(c.name), c.summary, c.schema))
        .chain(std::iter::once(tool(
            SCRIPT_TOOL.to_owned(),
            "Run a JavaScript script as one undo step, for many edits at once. It answers the script's last value and what it printed",
            SCRIPT_SCHEMA,
        )))
        .collect()
}

fn call(params: &Value, host: Result<&dyn Host, &str>) -> Result<Value, Failure> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| (-32602, "tools/call needs a tool name.".to_owned(), None))?;
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let command = COMMANDS.iter().find(|c| tool_name(c.name) == name);
    if command.is_none() && name != SCRIPT_TOOL {
        return Err((-32602, format!("Unknown tool: {name}"), None));
    }
    let done = match (host, command) {
        (Err(why), _) => Err(why.to_owned()),
        // Saves, footage and exports keep to the project's folder.
        (Ok(host), Some(command)) => {
            commands::run(host, Some(&Reach::project()), command.name, args)
                .map_err(|e| e.to_string())
        }
        (Ok(host), None) => script(host, &args),
    };
    // A frame comes with its picture, so the client can look at it.
    let picture = match (&done, command) {
        (Ok(value), Some(command)) if command.name == "comp.frame" => picture(value),
        _ => None,
    };
    // A command that did nothing is the tool's answer, not the protocol's,
    // so the client reads why and can try again.
    let (text, failed) = match done {
        Ok(value) => (value.to_string(), false),
        Err(why) => (why, true),
    };
    let content: Vec<Value> = std::iter::once(json!({ "type": "text", "text": text }))
        .chain(picture)
        .collect();
    Ok(json!({ "content": content, "isError": failed }))
}

/// The PNG `comp.frame` answered with, as an image for the client, when it
/// is no bigger than [`MAX_PICTURE`].
fn picture(answer: &Value) -> Option<Value> {
    let path = std::path::Path::new(answer.get("path")?.as_str()?);
    if std::fs::metadata(path).ok()?.len() > MAX_PICTURE {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(json!({ "type": "image", "data": base64(&bytes), "mimeType": "image/png" }))
}

/// `bytes` in standard base64, with padding.
fn base64(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            let letter = LETTERS[((n >> (18 - 6 * i)) & 63) as usize];
            out.push(if i <= chunk.len() {
                char::from(letter)
            } else {
                '='
            });
        }
    }
    out
}

/// Run `script_run`'s source. Nobody is there to choose a file, and the
/// script has no folder of its own, so it reads no files at all.
fn script(host: &dyn Host, args: &Value) -> Result<Value, String> {
    let source = args
        .get("source")
        .and_then(Value::as_str)
        .ok_or("script_run needs the script as source.")?;
    let mut options = RunOptions {
        stop: host.stop_flag(),
        limits: Limits::Project,
        ..RunOptions::default()
    };
    if let Some(name) = args.get("name").and_then(Value::as_str) {
        options.name = name.to_owned();
    }
    let report = run(host, source, options);
    match report.result {
        Ok(value) => Ok(json!({ "value": value, "log": report.log })),
        Err(e) => {
            let mut why = e.to_string();
            for line in &report.log {
                why.push('\n');
                why.push_str(line);
            }
            Err(why)
        }
    }
}
