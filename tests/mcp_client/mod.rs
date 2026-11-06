//! A small JSON-RPC over HTTP client for the MCP Controller, shared by the
//! tests that drive it: plain reqwest, so no MCP client crate is needed.
#![allow(dead_code)]

use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// A port no listener holds right now, for an MCP Controller in a test: the
/// MCP Controller, like telnet and WebSocket, refuses port 0, so a test binds
/// an ephemeral port, reads it, and frees it for Goose.
pub fn free_port() -> u16 {
    free_ports(1)[0]
}

/// `n` distinct free ports, each held until all are found, so no two are the
/// same.
pub fn free_ports(n: usize) -> Vec<u16> {
    let held: Vec<_> = (0..n)
        .map(|_| std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port"))
        .collect();
    held.iter()
        .map(|listener| listener.local_addr().expect("local address").port())
        .collect()
}

/// Fail at once, saying why, when `GOOSE_MCP_TOKEN` is set where the tests
/// run: every Goose they start with `--mcp` would take it as its token.
pub fn no_token_in_env() {
    assert!(
        std::env::var_os("GOOSE_MCP_TOKEN").is_none(),
        "unset GOOSE_MCP_TOKEN to run these tests: Goose reads it as the MCP token"
    );
}

/// The answer to one HTTP request: its status and body.
pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    /// The body as JSON.
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("json body: {}: {}", e, self.body))
    }
}

/// A tool call's result: whether it is not an error, its text, and its
/// structured content.
#[derive(Debug)]
pub struct Called {
    pub ok: bool,
    pub text: String,
    pub structured: Value,
}

/// A client of one MCP Controller.
pub struct Mcp {
    client: reqwest::Client,
    pub url: String,
    token: Option<String>,
    next_id: u64,
}

impl Mcp {
    /// A client of the MCP Controller on 127.0.0.1:`port`.
    pub fn new(port: u16) -> Self {
        no_token_in_env();
        Mcp {
            // Every request ends within 30 seconds: a tool answers within 15.
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("client"),
            url: format!("http://127.0.0.1:{port}/mcp"),
            token: None,
            next_id: 0,
        }
    }

    /// The same client, sending `token` as its Bearer token.
    pub fn with_token(mut self, token: &str) -> Self {
        self.token = Some(token.to_string());
        self
    }

    /// POST `body` with `headers` added, retrying for up to 20 seconds while
    /// nothing listens yet.
    pub async fn post(&self, body: &str, headers: &[(&str, &str)]) -> Reply {
        self.reach(body, headers)
            .await
            .unwrap_or_else(|| panic!("POST {}: nothing listened within 20 seconds", self.url))
    }

    /// As [`Mcp::post`], but `None` when nothing listened on the port within
    /// 20 seconds, as when Goose could not bind it.
    pub async fn reach(&self, body: &str, headers: &[(&str, &str)]) -> Option<Reply> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let mut request = self
                .client
                .post(&self.url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .body(body.to_string());
            if let Some(token) = &self.token {
                request = request.bearer_auth(token);
            }
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body = response.text().await.expect("body");
                    return Some(Reply { status, body });
                }
                Err(e) if e.is_connect() && Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) if e.is_connect() => return None,
                Err(e) => panic!("POST {}: {}", self.url, e),
            }
        }
    }

    /// Send one JSON-RPC request and return its whole JSON-RPC response.
    pub async fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let body =
            json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params });
        let reply = self.post(&body.to_string(), &[]).await;
        assert_eq!(reply.status, 200, "{}: {}", method, reply.body);
        reply.json()
    }

    /// The tools `tools/list` lists.
    pub async fn tools(&mut self) -> Vec<Value> {
        let reply = self.rpc("tools/list", json!({})).await;
        reply["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools/list: {}", reply))
            .clone()
    }

    /// Call `tool` with `arguments`; panics on a JSON-RPC error.
    pub async fn call(&mut self, tool: &str, arguments: Value) -> Called {
        let reply = self
            .rpc(
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
            )
            .await;
        let result = &reply["result"];
        assert!(result.is_object(), "{} gave no result: {}", tool, reply);
        Called {
            ok: result["isError"] != json!(true),
            text: result["content"][0]["text"]
                .as_str()
                .unwrap_or_else(|| panic!("{} gave no text: {}", tool, reply))
                .to_string(),
            structured: result["structuredContent"].clone(),
        }
    }

    /// Send the Controller line a telnet user types, as the tool call that
    /// is its MCP form, and return its decision and message.
    pub async fn send_line(&mut self, line: &str) -> (bool, String) {
        let (tool, arguments) =
            tool_for_line(line).unwrap_or_else(|| panic!("{:?} has no MCP tool", line));
        let called = self.call(tool, arguments).await;
        (called.ok, called.text)
    }
}

/// The MCP tool and arguments for a Controller line: the same verbs and
/// aliases the line parser takes (a copy, as a test can not reach the
/// crate's own table; tests/controller.rs and control_agreement.rs fail if
/// it falls behind the lines they send), with the value as the tool's argument; a
/// number-typed argument is sent as a JSON number when the value is one.
pub fn tool_for_line(line: &str) -> Option<(&'static str, Value)> {
    let line = line.trim();
    let (verb, value) = line.split_once(' ').unwrap_or((line, ""));
    // An integer too large for a u64 stays text, so it reaches the parser
    // as typed rather than as a float.
    let number = || {
        let digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
        if digits && value.parse::<u64>().is_err() {
            return Value::String(value.to_string());
        }
        serde_json::from_str::<serde_json::Number>(value)
            .map(Value::Number)
            .unwrap_or_else(|_| Value::String(value.to_string()))
    };
    let text = || Value::String(value.to_string());
    // The saved run commands: a run's name, or for compare a run and a
    // baseline, each left out when not given.
    let named = |pairs: &[(&str, Option<&str>)]| {
        let mut arguments = serde_json::Map::new();
        for (name, value) in pairs {
            if let Some(value) = value.filter(|value| !value.is_empty()) {
                arguments.insert(name.to_string(), Value::String(value.to_string()));
            }
        }
        Value::Object(arguments)
    };
    let run = (!value.trim().is_empty()).then(|| value.trim());
    match verb.to_ascii_lowercase().as_str() {
        "runs" | "runsjson" | "runs-json" => {
            return Some(("list_runs", named(&[("before", run)])));
        }
        "report" => return Some(("get_run_report", named(&[("run", run)]))),
        "reportjson" | "report-json" => {
            return Some((
                "get_run_report",
                named(&[("run", run), ("format", Some("json"))]),
            ));
        }
        "compare" => {
            let names: Vec<&str> = value.split_whitespace().collect();
            let (run, baseline) = match names[..] {
                [baseline] => (None, Some(baseline)),
                [run, baseline] => (Some(run), Some(baseline)),
                _ => (None, None),
            };
            return Some((
                "compare_runs",
                named(&[("baseline", baseline), ("run", run)]),
            ));
        }
        _ => {}
    }
    let (tool, argument) = match verb.to_ascii_lowercase().as_str() {
        "start" => ("start", None),
        "stop" => ("stop", None),
        "shutdown" => ("shutdown", None),
        "config" | "configjson" | "config-json" => ("get_config", None),
        "metrics" | "stats" | "metricsjson" | "metrics-json" | "statsjson" | "stats-json" => {
            ("get_metrics", None)
        }
        "host" | "hostname" | "host_name" | "host-name" => ("set_host", Some(("host", text()))),
        "users" | "user" => ("set_users", Some(("users", number()))),
        "increaserate" | "increase_rate" | "increase-rate" => {
            ("increase_rate", Some(("rate", number())))
        }
        "decreaserate" | "decrease_rate" | "decrease-rate" => {
            ("decrease_rate", Some(("rate", number())))
        }
        "increasetime" | "increase_time" | "increase-time" => {
            ("increase_time", Some(("time", text())))
        }
        "decreasetime" | "decrease_time" | "decrease-time" => {
            ("decrease_time", Some(("time", text())))
        }
        "run" | "runtime" | "run_time" | "run-time" => ("set_run_time", Some(("time", text()))),
        "testplan" | "test_plan" | "test-plan" | "plan" => {
            ("set_test_plan", Some(("plan", text())))
        }
        "throttle" | "throttle_requests" | "throttle-requests" => {
            ("set_throttle", Some(("requests", number())))
        }
        _ => return None,
    };
    let mut arguments = serde_json::Map::new();
    if let Some((name, value)) = argument {
        arguments.insert(name.to_string(), value);
    }
    Some((tool, Value::Object(arguments)))
}
