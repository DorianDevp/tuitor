//! JSON-RPC 2.0 over a pair of byte streams, each message framed with a `Content-Length`
//! header as DAP frames its own.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::{Result, anyhow, bail};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{FramedRead, FramedWrite};
use tuitor_dap::DapCodec;

/// What the server sends that no request of ours waits for.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    /// A request of the server's own, answered with [`Rpc::respond`].
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

type Reply = oneshot::Sender<Result<Value, String>>;
type Pending = Arc<Mutex<HashMap<i64, Reply>>>;

/// The connection; clones share it.
#[derive(Clone)]
pub struct Rpc {
    out: mpsc::UnboundedSender<Bytes>,
    pending: Pending,
    next: Arc<AtomicI64>,
}

impl Rpc {
    /// Talks over `reader` and `writer`. What the server sends of its own comes on the
    /// receiver, which ends when the server closes its side.
    pub fn start(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> (Rpc, mpsc::UnboundedReceiver<Incoming>) {
        let (out, mut outgoing) = mpsc::unbounded_channel::<Bytes>();
        let (tx, rx) = mpsc::unbounded_channel();
        let pending = Pending::default();
        tokio::spawn(async move {
            let mut sink = FramedWrite::new(writer, DapCodec::default());
            while let Some(body) = outgoing.recv().await {
                if sink.send(body).await.is_err() {
                    break;
                }
            }
        });
        let replies = pending.clone();
        tokio::spawn(async move {
            let mut frames = FramedRead::new(reader, DapCodec::default());
            while let Some(Ok(frame)) = frames.next().await {
                match serde_json::from_slice::<Value>(&frame) {
                    Ok(message) => dispatch(message, &replies, &tx),
                    Err(err) => tracing::warn!("lsp: a message that is no JSON: {err}"),
                }
            }
            // Requests still waiting learn that no answer comes.
            replies.lock().unwrap_or_else(PoisonError::into_inner).clear();
        });
        (Rpc { out, pending, next: Arc::new(AtomicI64::new(1)) }, rx)
    }

    /// Sends a request and waits for its result; an error the server answers with is one too.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap_or_else(PoisonError::into_inner).insert(id, tx);
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        match rx.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(message)) => bail!("{method}: {message}"),
            Err(_) => bail!("{method}: the language server closed the connection"),
        }
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    /// Answers a request of the server's: a result, or an error code and message.
    pub fn respond(&self, id: Value, result: Result<Value, (i64, String)>) -> Result<()> {
        let message = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        };
        self.send(&message)
    }

    fn send(&self, message: &Value) -> Result<()> {
        let body = Bytes::from(serde_json::to_vec(message)?);
        self.out.send(body).map_err(|_| anyhow!("the language server closed the connection"))
    }
}

/// A response goes to the request that waits for it; anything else to `incoming`.
fn dispatch(mut message: Value, pending: &Pending, incoming: &mpsc::UnboundedSender<Incoming>) {
    let method = message.get("method").and_then(Value::as_str).map(str::to_owned);
    let params = message.get_mut("params").map(Value::take).unwrap_or(Value::Null);
    match (method, message.get("id").cloned()) {
        (Some(method), Some(id)) => drop(incoming.send(Incoming::Request { id, method, params })),
        (Some(method), None) => drop(incoming.send(Incoming::Notification { method, params })),
        (None, Some(id)) => {
            let reply = id.as_i64().and_then(|id| pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&id));
            let Some(reply) = reply else { return };
            let result = match message.get("error") {
                Some(error) => Err(error.get("message").and_then(Value::as_str).unwrap_or("failed").to_owned()),
                None => Ok(message.get_mut("result").map(Value::take).unwrap_or(Value::Null)),
            };
            // Nobody waits any more when the request was given up on.
            let _ = reply.send(result);
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio_util::codec::{Decoder, Encoder};

    use super::*;

    /// The other end of the connection: reads one message, writes frames.
    struct Peer {
        stream: DuplexStream,
        buf: BytesMut,
        codec: DapCodec,
    }

    impl Peer {
        async fn read(&mut self) -> Value {
            loop {
                if let Some(frame) = self.codec.decode(&mut self.buf).expect("frame") {
                    return serde_json::from_slice(&frame).expect("json");
                }
                let mut chunk = [0u8; 1024];
                let n = self.stream.read(&mut chunk).await.expect("read");
                assert!(n > 0, "closed");
                self.buf.extend_from_slice(&chunk[..n]);
            }
        }

        async fn write(&mut self, message: Value) {
            let mut out = BytesMut::new();
            let body = Bytes::from(serde_json::to_vec(&message).expect("json"));
            self.codec.encode(body, &mut out).expect("encode");
            self.stream.write_all(&out).await.expect("write");
        }
    }

    fn connect() -> (Rpc, mpsc::UnboundedReceiver<Incoming>, Peer) {
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        let (reader, writer) = tokio::io::split(ours);
        let (rpc, incoming) = Rpc::start(reader, writer);
        (rpc, incoming, Peer { stream: theirs, buf: BytesMut::new(), codec: DapCodec::default() })
    }

    #[tokio::test]
    async fn requests_get_their_own_answers_and_the_rest_comes_in_order() {
        let (rpc, mut incoming, mut peer) = connect();
        let first = tokio::spawn({
            let rpc = rpc.clone();
            async move { rpc.request("textDocument/references", json!({})).await }
        });
        let request = peer.read().await;
        assert_eq!(request["method"], "textDocument/references");
        // The server asks something and says something before it answers.
        peer.write(
            json!({ "jsonrpc": "2.0", "id": "c1", "method": "workspace/configuration", "params": { "items": [{}] } }),
        )
        .await;
        peer.write(json!({ "jsonrpc": "2.0", "method": "$/progress", "params": { "token": 1 } })).await;
        peer.write(json!({ "jsonrpc": "2.0", "id": request["id"], "result": [1, 2] })).await;
        assert_eq!(first.await.expect("task").expect("result"), json!([1, 2]));
        assert_eq!(
            incoming.recv().await,
            Some(Incoming::Request {
                id: json!("c1"),
                method: "workspace/configuration".into(),
                params: json!({ "items": [{}] })
            })
        );
        assert_eq!(
            incoming.recv().await,
            Some(Incoming::Notification { method: "$/progress".into(), params: json!({ "token": 1 }) })
        );

        rpc.respond(json!("c1"), Ok(json!([null]))).expect("respond");
        assert_eq!(peer.read().await, json!({ "jsonrpc": "2.0", "id": "c1", "result": [null] }));
    }

    #[tokio::test]
    async fn an_error_answer_and_a_closed_connection_are_errors() {
        let (rpc, _incoming, mut peer) = connect();
        let asked = tokio::spawn({
            let rpc = rpc.clone();
            async move { rpc.request("callHierarchy/incomingCalls", json!({})).await }
        });
        let request = peer.read().await;
        let error = json!({ "code": -32601, "message": "Unhandled method" });
        peer.write(json!({ "jsonrpc": "2.0", "id": request["id"], "error": error })).await;
        let err = asked.await.expect("task").expect_err("an error");
        assert_eq!(err.to_string(), "callHierarchy/incomingCalls: Unhandled method");

        let waiting = tokio::spawn({
            let rpc = rpc.clone();
            async move { rpc.request("shutdown", Value::Null).await }
        });
        peer.read().await;
        drop(peer);
        let err = waiting.await.expect("task").expect_err("closed");
        assert!(err.to_string().contains("closed the connection"), "{err}");
    }
}
