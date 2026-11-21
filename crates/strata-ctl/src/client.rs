//! Synchronous NDJSON JSON-RPC 2.0 Unix domain socket client for `stratad`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use strata_protocol::rpc::{
    TaskExitPayload, TaskOutputPayload, TaskRunParams, TaskRunResult, TaskStream, events, methods,
};
use strata_protocol::{EventEnvelope, RpcRequest, RpcResponse};

static NEXT_REQ_ID: AtomicU64 = AtomicU64::new(1);

/// Connected NDJSON client talking to `/data/adb/strata/run/strata.sock`.
#[derive(Debug)]
pub(crate) struct StrataClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl StrataClient {
    /// Connects to `socket_path`.
    pub(crate) fn connect(socket_path: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(socket_path).map_err(|e| {
            format!(
                "cannot connect to stratad at '{}': {e} (is stratad running?)",
                socket_path.display()
            )
        })?;
        let writer = stream
            .try_clone()
            .map_err(|e| format!("clone socket stream: {e}"))?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    /// Sends one JSON-RPC 2.0 request and waits for its matching response frame.
    pub(crate) fn call(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, (i32, String)> {
        let req_id = format!("cli-{}", NEXT_REQ_ID.fetch_add(1, Ordering::Relaxed));
        let req = RpcRequest::new(req_id.clone(), method, params);
        let payload =
            serde_json::to_string(&req).map_err(|e| (-32000, format!("encode request: {e}")))?;

        self.writer
            .write_all(payload.as_bytes())
            .and_then(|()| self.writer.write_all(b"\n"))
            .and_then(|()| self.writer.flush())
            .map_err(|e| (-32000, format!("write socket: {e}")))?;

        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .reader
                .read_line(&mut line)
                .map_err(|e| (-32000, format!("read socket: {e}")))?;
            if n == 0 {
                return Err((-32000, "stratad closed connection unexpectedly".to_owned()));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(resp) = serde_json::from_str::<RpcResponse>(trimmed)
                && resp.id == req_id
            {
                if let Some(err) = resp.error {
                    return Err((err.code, err.message));
                }
                return Ok(resp.result.unwrap_or(Value::Null));
            }
        }
    }

    /// Invokes an RPC method and deserializes its typed result payload.
    pub(crate) fn call_typed<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<T, (i32, String)> {
        let val = self.call(method, params)?;
        serde_json::from_value(val).map_err(|e| (-32000, format!("decode '{method}' result: {e}")))
    }

    /// Subscribes to daemon events, launches a headless streaming task, prints stdout/stderr live,
    /// and returns the task exit code.
    pub(crate) fn run_streaming_task(
        &mut self,
        params: TaskRunParams,
        json_mode: bool,
    ) -> Result<i32, String> {
        self.call(methods::EVENTS_SUBSCRIBE, None)
            .map_err(|(c, m)| format!("error {c}: {m}"))?;
        let started: TaskRunResult = self
            .call_typed(
                methods::TASK_RUN,
                Some(serde_json::to_value(&params).map_err(|e| e.to_string())?),
            )
            .map_err(|(c, m)| format!("error {c}: {m}"))?;

        if json_mode && let Ok(text) = serde_json::to_string(&json!({ "started": started })) {
            println!("{text}");
        }

        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .reader
                .read_line(&mut line)
                .map_err(|e| format!("read event stream: {e}"))?;
            if n == 0 {
                return Err("event stream closed before task exit".to_owned());
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(evt) = serde_json::from_str::<EventEnvelope>(trimmed) else {
                continue;
            };
            match (evt.event.as_str(), evt.payload) {
                (events::TASK_OUTPUT, Some(payload)) => {
                    if let Ok(out) = serde_json::from_value::<TaskOutputPayload>(payload)
                        && out.task_id == started.task_id
                    {
                        if json_mode {
                            println!("{trimmed}");
                        } else {
                            match out.stream {
                                TaskStream::Stdout => println!("{}", out.chunk),
                                TaskStream::Stderr => eprintln!("{}", out.chunk),
                            }
                        }
                    }
                }
                (events::TASK_EXIT, Some(payload)) => {
                    if let Ok(exit) = serde_json::from_value::<TaskExitPayload>(payload)
                        && exit.task_id == started.task_id
                    {
                        if json_mode {
                            println!("{trimmed}");
                        }
                        return Ok(exit.exit_code);
                    }
                }
                _ => {}
            }
        }
    }
}
