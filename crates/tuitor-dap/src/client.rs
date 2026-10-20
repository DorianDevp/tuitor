//! Async DAP client: owns the adapter process (if any), correlates responses with
//! requests by `seq` and forwards events and reverse requests to the caller.

use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::codec::DapCodec;
use crate::events::Event;
use crate::protocol::{ProtocolMessage, RequestMessage, ResponseMessage};
use crate::requests::Request;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot start adapter `{program}`: {source}")]
    Spawn { program: String, source: std::io::Error },
    #[error("cannot connect to adapter at {addr}: {source}")]
    Connect { addr: String, source: std::io::Error },
    #[error("adapter connection closed")]
    Closed,
    #[error("{command}: {message}")]
    Failed { command: String, message: String },
    #[error("{command}: unexpected response body: {source}")]
    Decode { command: String, source: serde_json::Error },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// How to start or reach a debug adapter.
#[derive(Debug, Clone)]
pub enum Transport {
    /// The adapter speaks DAP on its stdin/stdout.
    Stdio(AdapterCommand),
    /// The adapter listens on TCP; `${port}` in its arguments is replaced with a free port.
    Server(AdapterCommand),
    /// Something is already listening at `host:port`.
    Tcp(String),
}

#[derive(Debug, Clone)]
pub struct AdapterCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl AdapterCommand {
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            .envs(self.env.iter().cloned())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // A process group of its own, out of the terminal's: ctrl-c there and the terminal
        // closing signal its whole foreground group. The caller then ends the adapter in
        // order; signalled too, js-debug would leave the browser it started to close by itself,
        // still writing its profile after the caller removed the adapter's TMPDIR.
        #[cfg(unix)]
        cmd.process_group(0);
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        cmd
    }

    fn spawn(&self) -> Result<Child, Error> {
        self.command().spawn().map_err(|source| Error::Spawn { program: self.program.clone(), source })
    }
}

/// Everything the adapter sends that no caller of [`Client::call`] waits for. Protocol
/// messages keep the order the adapter sent them in.
#[derive(Debug)]
pub enum Incoming {
    Event(Event),
    /// Reverse request (`runInTerminal`, `startDebugging`); answer with [`Client::respond`].
    Request(RequestMessage),
    /// Response to a request sent with [`Client::call_in_stream`].
    Response(ResponseMessage),
    /// A line the adapter process wrote outside the protocol (stderr, or stdout for TCP adapters).
    Log(String),
    /// The connection is gone; no more messages will follow.
    Closed,
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    out: mpsc::UnboundedSender<Bytes>,
    pending: Mutex<HashMap<i64, Reply>>,
    seq: AtomicI64,
    child: Mutex<Option<Child>>,
    /// `host:port` of an adapter reached over TCP.
    address: Option<String>,
}

/// Where the response to a request goes.
enum Reply {
    Caller(oneshot::Sender<ResponseMessage>),
    /// Into the [`Incoming`] stream, between the events sent before and after it.
    Stream,
}

impl Client {
    pub async fn connect(transport: Transport) -> Result<(Client, mpsc::UnboundedReceiver<Incoming>), Error> {
        let (tx, rx) = mpsc::unbounded_channel();
        let client = match transport {
            Transport::Stdio(cmd) => {
                let mut child = cmd.spawn()?;
                let (Some(stdin), Some(stdout), Some(stderr)) =
                    (child.stdin.take(), child.stdout.take(), child.stderr.take())
                else {
                    return Err(Error::Closed);
                };
                forward_lines(stderr, tx.clone());
                Client::from_io(stdout, stdin, Some(child), None, tx)
            }
            Transport::Server(cmd) => {
                let port = free_port()?;
                let mut cmd = cmd;
                for arg in &mut cmd.args {
                    *arg = arg.replace("${port}", &port.to_string());
                }
                let mut child = cmd.spawn()?;
                if let Some(stdout) = child.stdout.take() {
                    forward_lines(stdout, tx.clone());
                }
                if let Some(stderr) = child.stderr.take() {
                    forward_lines(stderr, tx.clone());
                }
                let addr = format!("127.0.0.1:{port}");
                let stream = connect_with_retry(&addr).await?;
                let (read, write) = stream.into_split();
                Client::from_io(read, write, Some(child), Some(addr), tx)
            }
            Transport::Tcp(addr) => {
                let stream = connect_with_retry(&addr).await?;
                let (read, write) = stream.into_split();
                Client::from_io(read, write, None, Some(addr), tx)
            }
        };
        Ok((client, rx))
    }

    fn from_io<R, W>(
        reader: R,
        writer: W,
        child: Option<Child>,
        address: Option<String>,
        incoming: mpsc::UnboundedSender<Incoming>,
    ) -> Client
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Bytes>();
        let inner = Arc::new(Inner {
            out: out_tx,
            pending: Mutex::new(HashMap::new()),
            seq: AtomicI64::new(1),
            child: Mutex::new(child),
            address,
        });

        tokio::spawn(async move {
            let mut sink = FramedWrite::new(writer, DapCodec::default());
            while let Some(frame) = out_rx.recv().await {
                if let Err(err) = sink.send(frame).await {
                    tracing::warn!("dap write failed: {err}");
                    break;
                }
            }
        });

        let weak = Arc::downgrade(&inner);
        tokio::spawn(read_loop(FramedRead::new(reader, DapCodec::default()), weak, incoming));

        Client { inner }
    }

    /// Sends a typed request and waits for its response.
    pub async fn request<R: Request>(&self, args: R::Arguments) -> Result<R::Response, Error> {
        self.call::<R>(args)?.await
    }

    /// Writes the request immediately and returns a future for its response.
    ///
    /// Requests are written in call order, which matters during configuration
    /// (`launch` before `setBreakpoints` before `configurationDone`).
    pub fn call<R: Request>(&self, args: R::Arguments) -> Result<Pending<R::Response>, Error> {
        let rx = self.start(R::COMMAND, serde_json::to_value(args)?)?;
        Ok(Pending { command: R::COMMAND, rx, _body: PhantomData })
    }

    /// Writes the request immediately, like [`Client::call`], but its response arrives as
    /// [`Incoming::Response`], after the events the adapter sent before it. A response
    /// awaited in another task can overtake those events on the way to the caller.
    pub fn call_in_stream<R: Request>(&self, args: R::Arguments) -> Result<(), Error> {
        self.send_request(R::COMMAND, serde_json::to_value(args)?, Reply::Stream)
    }

    /// Sends a request with arbitrary arguments; fails on `success: false`.
    pub async fn request_raw(&self, command: &str, arguments: Value) -> Result<ResponseMessage, Error> {
        let response = self.start(command, arguments)?.await.map_err(|_| Error::Closed)?;
        check(command, response)
    }

    fn start(&self, command: &str, arguments: Value) -> Result<oneshot::Receiver<ResponseMessage>, Error> {
        let (tx, rx) = oneshot::channel();
        self.send_request(command, arguments, Reply::Caller(tx))?;
        Ok(rx)
    }

    fn send_request(&self, command: &str, arguments: Value, reply: Reply) -> Result<(), Error> {
        let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed);
        self.inner.pending().insert(seq, reply);

        let msg = ProtocolMessage::Request(RequestMessage { seq, command: command.to_owned(), arguments });
        if let Err(err) = self.send(&msg) {
            self.inner.pending().remove(&seq);
            return Err(err);
        }
        Ok(())
    }

    /// Answers a reverse request received as [`Incoming::Request`].
    pub fn respond(&self, request: &RequestMessage, success: bool, body: Value) -> Result<(), Error> {
        self.send_response(request, success, None, body)
    }

    /// Turns down a reverse request, saying why in the response's `message`.
    pub fn respond_error(&self, request: &RequestMessage, message: &str) -> Result<(), Error> {
        self.send_response(request, false, Some(message.to_owned()), Value::Null)
    }

    fn send_response(
        &self,
        request: &RequestMessage,
        success: bool,
        message: Option<String>,
        body: Value,
    ) -> Result<(), Error> {
        let msg = ProtocolMessage::Response(ResponseMessage {
            seq: self.inner.seq.fetch_add(1, Ordering::Relaxed),
            request_seq: request.seq,
            success,
            command: request.command.clone(),
            message,
            body,
        });
        self.send(&msg)
    }

    /// `host:port` of an adapter reached over TCP (`Server` or `Tcp` transport). An adapter
    /// that asks for child sessions with `startDebugging` serves them on the same address.
    pub fn address(&self) -> Option<&str> {
        self.inner.address.as_deref()
    }

    /// Kills the adapter process, if this client started one.
    pub fn kill(&self) {
        let mut child = self.inner.child.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(child) = child.as_mut() {
            let _ = child.start_kill();
        }
    }

    /// Kills the adapter process, if this client started one, and waits until it has exited.
    pub async fn kill_and_wait(&self) {
        let child = self.inner.child.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(mut child) = child {
            // An error means it had exited already.
            let _ = child.kill().await;
        }
    }

    fn send(&self, msg: &ProtocolMessage) -> Result<(), Error> {
        let bytes = serde_json::to_vec(msg)?;
        tracing::trace!(target: "dap", "→ {}", String::from_utf8_lossy(&bytes));
        self.inner.out.send(bytes.into()).map_err(|_| Error::Closed)
    }
}

impl Inner {
    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Reply>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

async fn read_loop<R>(
    mut frames: FramedRead<R, DapCodec>,
    inner: Weak<Inner>,
    incoming: mpsc::UnboundedSender<Incoming>,
) where
    R: AsyncRead + Unpin,
{
    while let Some(frame) = frames.next().await {
        let frame = match frame {
            Ok(frame) => frame,
            Err(err) => {
                tracing::warn!("dap read failed: {err}");
                break;
            }
        };
        tracing::trace!(target: "dap", "← {}", String::from_utf8_lossy(&frame));
        let msg = match serde_json::from_slice::<ProtocolMessage>(&frame) {
            Ok(msg) => msg,
            Err(err) => {
                tracing::warn!("ignoring malformed dap message: {err}");
                continue;
            }
        };
        match msg {
            ProtocolMessage::Response(response) => {
                let Some(inner) = inner.upgrade() else { break };
                let reply = inner.pending().remove(&response.request_seq);
                match reply {
                    Some(Reply::Caller(tx)) => {
                        let _ = tx.send(response);
                    }
                    Some(Reply::Stream) => {
                        let _ = incoming.send(Incoming::Response(response));
                    }
                    None => {}
                }
            }
            ProtocolMessage::Event(event) => match Event::try_from(event) {
                Ok(event) => {
                    let _ = incoming.send(Incoming::Event(event));
                }
                Err(err) => tracing::warn!("ignoring malformed dap event: {err}"),
            },
            ProtocolMessage::Request(request) => {
                let _ = incoming.send(Incoming::Request(request));
            }
        }
    }
    if let Some(inner) = inner.upgrade() {
        // Dropping the senders fails every outstanding request with `Error::Closed`.
        inner.pending().clear();
    }
    let _ = incoming.send(Incoming::Closed);
}

fn forward_lines<R>(reader: R, incoming: mpsc::UnboundedSender<Incoming>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if incoming.send(Incoming::Log(line)).is_err() {
                break;
            }
        }
    });
}

/// Response to a request sent with [`Client::call`].
pub struct Pending<T> {
    command: &'static str,
    rx: oneshot::Receiver<ResponseMessage>,
    _body: PhantomData<fn() -> T>,
}

impl<T: DeserializeOwned> Future for Pending<T> {
    type Output = Result<T, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let command = self.command;
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(_)) => Poll::Ready(Err(Error::Closed)),
            Poll::Ready(Ok(response)) => {
                Poll::Ready(check(command, response).and_then(|r| decode_body(command, r.body)))
            }
        }
    }
}

fn check(command: &str, response: ResponseMessage) -> Result<ResponseMessage, Error> {
    if response.success {
        Ok(response)
    } else {
        Err(Error::Failed { command: command.to_owned(), message: failure_message(&response) })
    }
}

fn decode_body<T: DeserializeOwned>(command: &str, body: Value) -> Result<T, Error> {
    let body = if body.is_null() { Value::Object(Default::default()) } else { body };
    serde_json::from_value(body).map_err(|source| Error::Decode { command: command.to_owned(), source })
}

/// Prefers the structured `body.error.format` over the often generic `message`.
fn failure_message(response: &ResponseMessage) -> String {
    let format = response.body.pointer("/error/format").and_then(Value::as_str);
    match (format, &response.message) {
        (Some(format), _) => format.to_owned(),
        (None, Some(message)) => message.clone(),
        (None, None) => "request failed".to_owned(),
    }
}

fn free_port() -> Result<u16, Error> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

async fn connect_with_retry(addr: &str) -> Result<TcpStream, Error> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        match TcpStream::connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(source) if Instant::now() >= deadline => {
                return Err(Error::Connect { addr: addr.to_owned(), source });
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requests::{ConfigurationDone, Threads};
    use crate::types::Empty;

    /// A fake adapter on the other end of an in-memory pipe.
    async fn fake_adapter() -> (Client, mpsc::UnboundedReceiver<Incoming>, tokio::io::DuplexStream) {
        let (ours, theirs) = tokio::io::duplex(64 * 1024);
        let (read, write) = tokio::io::split(ours);
        let (tx, rx) = mpsc::unbounded_channel();
        (Client::from_io(read, write, None, None, tx), rx, theirs)
    }

    async fn reply(peer: &mut FramedRead<tokio::io::ReadHalf<tokio::io::DuplexStream>, DapCodec>) -> Value {
        let frame = peer.next().await.expect("frame").expect("valid frame");
        serde_json::from_slice(&frame).expect("json")
    }

    #[tokio::test]
    async fn correlates_responses_and_forwards_events() {
        let (client, mut incoming, peer) = fake_adapter().await;
        let (peer_read, peer_write) = tokio::io::split(peer);
        let mut peer_read = FramedRead::new(peer_read, DapCodec::default());
        let mut peer_write = FramedWrite::new(peer_write, DapCodec::default());

        let pending = tokio::spawn({
            let client = client.clone();
            async move { client.request::<Threads>(Empty {}).await }
        });

        let req = reply(&mut peer_read).await;
        assert_eq!(req["command"], "threads");
        let seq = req["seq"].as_i64().expect("seq");

        let event = serde_json::json!({"seq":1,"type":"event","event":"stopped","body":{"reason":"step","threadId":4}});
        peer_write.send(Bytes::from(event.to_string())).await.expect("send event");
        let response = serde_json::json!({
            "seq":2,"type":"response","request_seq":seq,"success":true,"command":"threads",
            "body":{"threads":[{"id":4,"name":"main"}]}
        });
        peer_write.send(Bytes::from(response.to_string())).await.expect("send response");

        let threads = pending.await.expect("join").expect("threads");
        assert_eq!(threads.threads[0].id, 4);
        assert!(matches!(incoming.recv().await, Some(Incoming::Event(Event::Stopped(b))) if b.thread_id == Some(4)));
    }

    #[tokio::test]
    async fn response_in_stream_keeps_its_place_among_events() {
        let (client, mut incoming, peer) = fake_adapter().await;
        let (peer_read, peer_write) = tokio::io::split(peer);
        let mut peer_read = FramedRead::new(peer_read, DapCodec::default());
        let mut peer_write = FramedWrite::new(peer_write, DapCodec::default());

        client.call_in_stream::<ConfigurationDone>(Empty {}).expect("send");
        let seq = reply(&mut peer_read).await["seq"].as_i64().expect("seq");
        let output = |seq: i64, text: &str| {
            let body = serde_json::json!({"category": "stdout", "output": text});
            serde_json::json!({"seq": seq, "type": "event", "event": "output", "body": body})
        };
        let done = serde_json::json!({
            "seq":2,"type":"response","request_seq":seq,"success":true,"command":"configurationDone"
        });
        for message in [output(1, "before"), done, output(3, "after")] {
            peer_write.send(Bytes::from(message.to_string())).await.expect("send");
        }

        let mut order = Vec::new();
        for _ in 0..3 {
            order.push(match incoming.recv().await {
                Some(Incoming::Event(Event::Output(body))) => body.output,
                Some(Incoming::Response(response)) => response.command,
                other => panic!("unexpected {other:?}"),
            });
        }
        assert_eq!(order, ["before", "configurationDone", "after"]);
    }

    #[tokio::test]
    async fn failed_response_uses_error_format() {
        let (client, _incoming, peer) = fake_adapter().await;
        let (peer_read, peer_write) = tokio::io::split(peer);
        let mut peer_read = FramedRead::new(peer_read, DapCodec::default());
        let mut peer_write = FramedWrite::new(peer_write, DapCodec::default());

        let pending = tokio::spawn({
            let client = client.clone();
            async move { client.request_raw("launch", Value::Null).await }
        });
        let seq = reply(&mut peer_read).await["seq"].as_i64().expect("seq");
        let response = serde_json::json!({
            "seq":1,"type":"response","request_seq":seq,"success":false,"command":"launch",
            "message":"error","body":{"error":{"id":1,"format":"port 9003 already in use"}}
        });
        peer_write.send(Bytes::from(response.to_string())).await.expect("send");

        let err = pending.await.expect("join").expect_err("must fail");
        assert_eq!(err.to_string(), "launch: port 9003 already in use");
    }

    #[tokio::test]
    async fn refuses_reverse_requests_with_a_reason() {
        let (client, mut incoming, peer) = fake_adapter().await;
        let (peer_read, peer_write) = tokio::io::split(peer);
        let mut peer_read = FramedRead::new(peer_read, DapCodec::default());
        let mut peer_write = FramedWrite::new(peer_write, DapCodec::default());

        let request = serde_json::json!({"seq": 9, "type": "request", "command": "runInTerminal", "arguments": {}});
        peer_write.send(Bytes::from(request.to_string())).await.expect("send");
        let Some(Incoming::Request(request)) = incoming.recv().await else { panic!("expected a reverse request") };
        client.respond_error(&request, "no terminal").expect("respond");

        let response = reply(&mut peer_read).await;
        assert_eq!(response["type"], "response");
        assert_eq!(response["request_seq"], 9);
        assert_eq!(response["success"], false);
        assert_eq!(response["message"], "no terminal");
    }

    #[tokio::test]
    async fn pending_requests_fail_when_adapter_goes_away() {
        let (client, mut incoming, peer) = fake_adapter().await;
        let pending = tokio::spawn({
            let client = client.clone();
            async move { client.request::<Threads>(Empty {}).await }
        });
        tokio::task::yield_now().await;
        drop(peer);
        assert!(matches!(pending.await.expect("join"), Err(Error::Closed)));
        assert!(matches!(incoming.recv().await, Some(Incoming::Closed)));
    }

    /// The process group in /proc/<pid>/stat, the fifth field: the second after the name.
    #[cfg(target_os = "linux")]
    fn process_group(pid: &str) -> String {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("stat");
        let (_, after_name) = stat.rsplit_once(')').expect("name");
        after_name.split_whitespace().nth(2).expect("pgrp").to_owned()
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn an_adapter_is_out_of_the_terminals_process_group() {
        let cmd = AdapterCommand { program: "cat".into(), args: vec![], cwd: None, env: vec![] };
        let (client, _incoming) = Client::connect(Transport::Stdio(cmd)).await.expect("cat starts");
        let pid = client.inner.child.lock().expect("lock").as_ref().and_then(Child::id).expect("pid").to_string();
        assert_eq!(process_group(&pid), pid, "a group of its own");
        assert_ne!(process_group(&pid), process_group("self"));
        client.kill_and_wait().await;
    }
}
