//! The Unix-socket RPC server: one JSON request per connection.

use std::sync::Arc;

use pinch_client::socket_path;
use pinch_proto::{PROTOCOL_VERSION, Request, Response};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::engine::Engine;

pub fn bind() -> anyhow::Result<UnixListener> {
    let path = socket_path();
    if path.exists() {
        // A live engine would have answered before we got here (see main), so
        // whatever is left is a stale socket from a crash.
        let _ = std::fs::remove_file(&path);
    }
    Ok(UnixListener::bind(path)?)
}

pub async fn serve(listener: UnixListener, engine: Arc<Engine>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let engine = Arc::clone(&engine);
        tokio::spawn(async move {
            let _ = handle(stream, engine).await;
        });
    }
}

async fn handle(stream: UnixStream, engine: Arc<Engine>) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    BufReader::new(read).read_line(&mut line).await?;

    let response = match serde_json::from_str::<Request>(&line) {
        // Engine calls can block (restart waits for the old process to die).
        Ok(request) => tokio::task::spawn_blocking(move || dispatch(&engine, request))
            .await
            .unwrap_or_else(|e| Response::Error {
                message: format!("internal error: {e}"),
            }),
        Err(e) => Response::Error {
            message: format!("bad request: {e}"),
        },
    };

    let mut out = serde_json::to_vec(&response).unwrap_or_default();
    out.push(b'\n');
    write.write_all(&out).await?;
    write.shutdown().await
}

fn dispatch(engine: &Arc<Engine>, request: Request) -> Response {
    let result: anyhow::Result<Response> = (|| {
        Ok(match request {
            Request::Hello { .. } => Response::Hello {
                version: PROTOCOL_VERSION,
            },
            Request::Ensure(spawn) => {
                let (process, already_running) = engine.ensure(spawn)?;
                Response::Ensured {
                    process,
                    already_running,
                }
            }
            Request::Run(spawn) => Response::Process {
                process: engine.run(spawn)?,
            },
            Request::Snapshot => {
                let (processes, decisions) = engine.snapshot();
                Response::Snapshot {
                    processes,
                    decisions,
                }
            }
            Request::Stop { name } => Response::Process {
                process: engine.stop(&name)?,
            },
            Request::Restart { name } => Response::Process {
                process: engine.restart(&name)?,
            },
            Request::Logs { name, tail } => Response::Logs {
                text: engine.logs(&name, tail.unwrap_or(200))?,
            },
            Request::Answer { decision, answer } => {
                engine.answer(&decision, answer)?;
                Response::Ok
            }
            Request::Shutdown => {
                // Reply first, then go; managed processes live on.
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    let _ = std::fs::remove_file(socket_path());
                    std::process::exit(0);
                });
                Response::Ok
            }
        })
    })();
    result.unwrap_or_else(|e| Response::Error {
        message: format!("{e:#}"),
    })
}
