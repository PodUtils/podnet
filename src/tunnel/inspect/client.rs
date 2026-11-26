//! The client of the socket of the inspector: one request, one answer.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

use crate::exit::{self, Fault};

/// The largest answer: a ring of large bodies in one `list` stays far
/// below it.
const ANSWER_LIMIT: u64 = 256 << 20;

/// The time limit of one request (a replay waits for the origin up to
/// 30 s).
const ASK_TIME: Duration = Duration::from_secs(45);

/// The exit class of the code of an error answer.
fn fault_of(code: i64) -> Fault {
    match code {
        64 => Fault::Usage,
        65 => Fault::DataError,
        66 => Fault::NoInput,
        71 => Fault::OsError,
        77 => Fault::NoPermission,
        124 => Fault::Timeout,
        _ => Fault::Unavailable,
    }
}

/// Send `request` to the inspector at `path`, and give its answer. An
/// error answer is an error with its exit code; a socket that nobody
/// serves exits 69.
pub async fn ask(path: &Path, request: &Value) -> Result<Value> {
    match tokio::time::timeout(ASK_TIME, exchange(path, request)).await {
        Ok(answer) => answer,
        Err(_) => Err(exit::fault(
            Fault::Timeout,
            anyhow!(
                "the inspector unix:{} gave no answer in 45 s",
                path.display()
            ),
        )),
    }
}

async fn exchange(path: &Path, request: &Value) -> Result<Value> {
    let gone = |e: std::io::Error| {
        exit::fault(
            Fault::Unavailable,
            anyhow!("the inspector unix:{}: {e}", path.display()),
        )
    };
    let mut stream = UnixStream::connect(path).await.map_err(gone)?;
    let mut line = request.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes()).await.map_err(gone)?;
    let mut got = Vec::new();
    (&mut stream)
        .take(ANSWER_LIMIT)
        .read_to_end(&mut got)
        .await
        .map_err(gone)?;
    let answer: Value = serde_json::from_slice(&got).map_err(|e| {
        exit::fault(
            Fault::Unavailable,
            anyhow!(
                "the inspector unix:{} gave no JSON answer: {e}",
                path.display()
            ),
        )
    })?;
    if answer["ok"].as_bool() == Some(true) {
        return Ok(answer);
    }
    let code = answer["code"].as_i64().unwrap_or(69);
    let text = answer["error"]
        .as_str()
        .unwrap_or("the inspector refused the request");
    Err(exit::fault(fault_of(code), anyhow!("{text}")))
}
