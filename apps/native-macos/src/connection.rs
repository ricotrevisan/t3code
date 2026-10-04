//! Bearer authorization and Effect RPC shell subscription on a dedicated runtime.
//! Credentials stay off argv, logs and persisted app state.
use crate::protocol::{self, ClientMessage, RequestId, RpcExit, ServerMessage};
use crate::reconnect::{Backoff, Failure};
use crate::shell::ShellItem;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio_tungstenite::tungstenite::Message;
use url::Url;

pub enum Update {
    Status(String),
    Resynchronizing,
    Shell(Box<ShellItem>),
    Thread {
        thread_id: String,
        item: Value,
    },
    Subscription {
        key: String,
        item: Result<Value, String>,
    },
}

pub enum Command {
    Subscribe {
        key: String,
        tag: String,
        payload: Value,
    },
    Unsubscribe(String),
    SelectThread(Option<String>),
    Call {
        tag: String,
        payload: Value,
        reply: async_channel::Sender<Result<Value, String>>,
    },
}

pub struct Session {
    pub updates: async_channel::Receiver<Update>,
    pub commands: async_channel::Sender<Command>,
}

impl Session {
    pub async fn call(&self, tag: &str, payload: Value) -> Result<Value, String> {
        call(&self.commands, tag, payload).await
    }
}

pub async fn call(
    commands: &async_channel::Sender<Command>,
    tag: &str,
    payload: Value,
) -> Result<Value, String> {
    let (reply, receiver) = async_channel::bounded(1);
    commands
        .send(Command::Call {
            tag: tag.into(),
            payload,
            reply,
        })
        .await
        .map_err(|_| "Not connected")?;
    receiver
        .recv()
        .await
        .map_err(|_| "Connection lost before receipt; RPC outcome is unknown".to_owned())?
}

pub struct ConnectionOptions {
    base: Url,
    token: String,
}

impl ConnectionOptions {
    pub fn origin(&self) -> String {
        self.base.origin().ascii_serialization()
    }
    pub fn new(base: &str, token: String) -> Result<Self, &'static str> {
        let base = Url::parse(base).map_err(|_| "Invalid server URL")?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err("Use an HTTP(S) origin without credentials, query or fragment");
        }
        if base.path() != "/" {
            return Err("Path-prefixed server URLs are not supported yet");
        }
        let loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if base.scheme() == "http" && !loopback {
            return Err("Remote servers require HTTPS");
        }
        if token.trim().is_empty() {
            return Err("A bearer access token is required");
        }
        Ok(Self { base, token })
    }
}

#[derive(Deserialize)]
struct Ticket {
    ticket: String,
}

pub fn start(options: ConnectionOptions) -> async_channel::Receiver<Update> {
    start_session(options).updates
}

pub fn start_session(options: ConnectionOptions) -> Session {
    // A bounded channel prevents a blocked UI from accumulating arbitrary history.
    let (sender, receiver) = async_channel::bounded(64);
    let (commands, requests) = async_channel::bounded(32);
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => {
                let _ = sender
                    .send_blocking(Update::Status("Could not start connection runtime".into()));
                return;
            }
        };
        runtime.block_on(async move {
            // Dropping the UI receiver cancels authorization, retries and the socket worker.
            tokio::select! {
                _ = supervise(options,&sender,&requests) => {}
                _ = sender.closed() => {}
            }
        });
    });
    Session {
        updates: receiver,
        commands,
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(options: &ConnectionOptions) -> Result<Socket, Failure> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| Failure::blocked("Could not create HTTP client"))?;
    let response = client
        .post(
            options
                .base
                .join("api/auth/websocket-ticket")
                .map_err(|_| Failure::blocked("Invalid ticket endpoint"))?,
        )
        .bearer_auth(&options.token)
        .send()
        .await
        .map_err(|_| Failure::transient("Could not reach authorization endpoint"))?;
    if !response.status().is_success() {
        if response.status().is_server_error() || response.status().as_u16() == 429 {
            return Err(Failure::transient(
                "Authorization endpoint temporarily unavailable",
            ));
        }
        return Err(Failure::blocked(
            "Authorization rejected; check token and scopes",
        ));
    }
    let ticket: Ticket = response
        .json()
        .await
        .map_err(|_| Failure::blocked("Invalid authorization response"))?;
    if ticket.ticket.trim().is_empty() {
        return Err(Failure::blocked("Invalid authorization response"));
    }
    let mut socket_url = options.base.clone();
    let scheme = if socket_url.scheme() == "https" {
        "wss"
    } else {
        "ws"
    };
    socket_url
        .set_scheme(scheme)
        .map_err(|_| Failure::blocked("Invalid WebSocket scheme"))?;
    socket_url.set_path("/ws");
    socket_url
        .query_pairs_mut()
        .append_pair("wsTicket", &ticket.ticket)
        .append_pair("orchestrationProtocol", "1")
        .append_pair("clientSurface", "desktop")
        .append_pair("clientDeviceType", "desktop")
        .append_pair("clientOs", "macOS")
        .append_pair("clientAppVersion", env!("CARGO_PKG_VERSION"))
        .append_pair("connectionMethod", "direct");
    let (socket, _) = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async(socket_url.as_str()),
    )
    .await
    .map_err(|_| Failure::transient("WebSocket connection timed out"))?
    .map_err(|error| match error {
        tokio_tungstenite::tungstenite::Error::Http(response)
            if response.status().is_client_error() && response.status().as_u16() != 429 =>
        {
            Failure::blocked("WebSocket authorization or protocol rejected")
        }
        _ => Failure::transient("WebSocket connection unavailable"),
    })?;
    Ok(socket)
}

type Subscriptions = HashMap<String, (String, Value)>;
fn supported_subscription(tag: &str) -> bool {
    matches!(tag, "subscribeProjectClones" | "subscribeWorktreeSetup")
}
async fn reject_disconnected(
    command: Command,
    selected: &mut Option<String>,
    subscriptions: &mut Subscriptions,
) {
    match command {
        Command::Subscribe { key, tag, payload } => {
            if supported_subscription(&tag) {
                subscriptions.insert(key, (tag, payload));
            }
        }
        Command::Unsubscribe(key) => {
            subscriptions.remove(&key);
        }
        Command::SelectThread(id) => *selected = id,
        Command::Call { reply, .. } => {
            let _ = reply
                .send(Err(
                    "Disconnected; request was not sent. Retry after synchronization.".into(),
                ))
                .await;
        }
    }
}

async fn supervise(
    options: ConnectionOptions,
    sender: &async_channel::Sender<Update>,
    requests: &async_channel::Receiver<Command>,
) {
    let mut selected = None;
    let mut subscriptions = Subscriptions::new();
    let mut backoff = Backoff::default();
    loop {
        let _ = sender.send(Update::Resynchronizing).await;
        let _ = sender.send(Update::Status("Authorizing…".into())).await;
        // Commands arriving during authorization must not wait and later execute unexpectedly.
        let connection = connect(&options);
        tokio::pin!(connection);
        let result = loop {
            tokio::select! {
                result=&mut connection => break result,
                command=requests.recv(), if !requests.is_closed() => { if let Ok(command)=command { reject_disconnected(command,&mut selected,&mut subscriptions).await; } }
            }
        };
        let failure = match result {
            Ok(socket) => {
                let _ = sender
                    .send(Update::Status("Loading projects and threads…".into()))
                    .await;
                let mut reached_sync = false;
                let result = run_selected_session(
                    socket,
                    sender,
                    requests,
                    &mut selected,
                    &mut reached_sync,
                    &mut subscriptions,
                )
                .await;
                if reached_sync {
                    backoff.reset();
                }
                match result {
                    Ok(()) => return,
                    Err(message) => match message {
                        "Unsupported or malformed RPC message"
                        | "Unsupported shell stream event"
                        | "Server rejected the RPC protocol"
                        | "Server reported an RPC defect"
                        | "Unexpected RPC request ID"
                        | "Empty RPC stream chunk"
                        | "Unary RPC returned a stream chunk" => Failure::blocked(message),
                        "Client closed" => return,
                        _ => Failure::transient(message),
                    },
                }
            }
            Err(failure) => failure,
        };
        // No queued mutation survives the disconnected epoch, regardless of whether it was read.
        while let Ok(command) = requests.try_recv() {
            reject_disconnected(command, &mut selected, &mut subscriptions).await;
        }
        if !failure.retryable {
            let _ = sender.send(Update::Status(failure.message.into())).await;
            return;
        }
        let delay = backoff.delay();
        let _ = sender.send(Update::Resynchronizing).await;
        let _ = sender
            .send(Update::Status(format!(
                "{} · reconnecting",
                failure.message
            )))
            .await;
        let wait = tokio::time::sleep(delay);
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _=&mut wait => break,
                command=requests.recv(), if !requests.is_closed() => { if let Ok(command)=command { reject_disconnected(command,&mut selected,&mut subscriptions).await; } }
            }
        }
    }
}

async fn send<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    message: ClientMessage,
) -> Result<(), &'static str>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let text = serde_json::to_string(&message).map_err(|_| "Could not encode RPC message")?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| "Connection closed while sending")?;
    if let ClientMessage::Request {
        id, tag, payload, ..
    } = &message
        && (tag.starts_with("orchestration.")
            || tag.starts_with("vcs.")
            || tag.starts_with("projects.")
            || tag.starts_with("projectClone.")
            || supported_subscription(tag))
        && let Some(path) = std::env::var_os("T3_NATIVE_EVIDENCE_TRACE")
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let record = json!({"direction":"outgoing","at":crate::commands::now(),"id":id,"tag":tag,"payload":payload});
        // Explicitly enabled for owned-fixture UI verification; never record auth traffic or headers.
        let result = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .and_then(|mut file| writeln!(file, "{record}"));
        if result.is_err() {
            eprintln!("Could not retain native RPC evidence");
        }
    }
    Ok(())
}

#[cfg(test)]
async fn run_socket<S>(
    socket: tokio_tungstenite::WebSocketStream<S>,
    sender: &async_channel::Sender<Update>,
) -> Result<(), &'static str>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (_commands, requests) = async_channel::bounded(1);
    run_session(socket, sender, &requests).await
}

enum Pending {
    Shell,
    Thread(String),
    Subscription(String),
    Call(async_channel::Sender<Result<Value, String>>),
}

#[cfg(test)]
async fn run_session<S>(
    socket: tokio_tungstenite::WebSocketStream<S>,
    sender: &async_channel::Sender<Update>,
    commands: &async_channel::Receiver<Command>,
) -> Result<(), &'static str>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    run_selected_session(
        socket,
        sender,
        commands,
        &mut None,
        &mut false,
        &mut Subscriptions::new(),
    )
    .await
}

async fn run_selected_session<S>(
    mut socket: tokio_tungstenite::WebSocketStream<S>,
    sender: &async_channel::Sender<Update>,
    commands: &async_channel::Receiver<Command>,
    selected: &mut Option<String>,
    reached_sync: &mut bool,
    subscriptions: &mut Subscriptions,
) -> Result<(), &'static str>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut pending = HashMap::from([(RequestId::String("1".into()), Pending::Shell)]);
    let mut cancelled = HashSet::new();
    let mut next_id = 2u64;
    let mut commands_open = true;
    let mut synchronized = false;
    send(
        &mut socket,
        protocol::request(
            1,
            "orchestration.subscribeShell",
            json!({"requestCompletionMarker":true}),
        ),
    )
    .await?;
    if let Some(thread_id) = selected.as_ref() {
        send(&mut socket,protocol::request(next_id,"orchestration.subscribeThread",json!({"threadId":thread_id,"reasoningMessages":true,"requestCompletionMarker":true}))).await?;
        pending.insert(
            RequestId::String(next_id.to_string()),
            Pending::Thread(thread_id.clone()),
        );
        next_id += 1;
    }
    for (key, (tag, payload)) in subscriptions.iter() {
        send(
            &mut socket,
            protocol::request(next_id, tag, payload.clone()),
        )
        .await?;
        pending.insert(
            RequestId::String(next_id.to_string()),
            Pending::Subscription(key.clone()),
        );
        next_id += 1;
    }
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(5),
        Duration::from_secs(5),
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pong_received = true;
    loop {
        tokio::select! {
            command = commands.recv(), if commands_open => {
                let command = match command { Ok(command) => command, Err(_) => { commands_open = false; continue; } };
                let id = RequestId::String(next_id.to_string());
                match command {
                    Command::Subscribe { key,tag,payload } => {
                        if !supported_subscription(&tag) {
                            sender.send(Update::Subscription { key, item:Err("Unsupported read subscription".into()) }).await.map_err(|_| "Client closed")?;
                            continue;
                        }
                        let old_ids: Vec<_> = pending.iter().filter_map(|(id,kind)| if matches!(kind,Pending::Subscription(old) if old == &key) { Some(id.clone()) } else { None }).collect();
                        for old in old_ids {
                            send(&mut socket,ClientMessage::Interrupt { request_id:old.clone() }).await?;
                            pending.remove(&old);cancelled.insert(old);
                        }
                        subscriptions.insert(key.clone(),(tag.clone(),payload.clone()));
                        send(&mut socket,protocol::request(next_id,&tag,payload)).await?;
                        pending.insert(id,Pending::Subscription(key));next_id += 1;
                    }
                    Command::Unsubscribe(key) => {
                        subscriptions.remove(&key);
                        let old_ids: Vec<_> = pending.iter().filter_map(|(id,kind)| if matches!(kind,Pending::Subscription(old) if old == &key) { Some(id.clone()) } else { None }).collect();
                        for old in old_ids {
                            send(&mut socket,ClientMessage::Interrupt { request_id:old.clone() }).await?;
                            pending.remove(&old);cancelled.insert(old);
                        }
                    }
                    Command::SelectThread(thread_id) => {
                        *selected=thread_id.clone();
                        let old_ids: Vec<_> = pending.iter().filter_map(|(id, kind)| if matches!(kind, Pending::Thread(_)) { Some(id.clone()) } else { None }).collect();
                        for old in old_ids {
                            send(&mut socket, ClientMessage::Interrupt { request_id: old.clone() }).await?;
                            pending.remove(&old);
                            cancelled.insert(old);
                        }
                        if let Some(thread_id) = thread_id {
                            send(&mut socket, protocol::request(next_id, "orchestration.subscribeThread", json!({"threadId":thread_id,"reasoningMessages":true,"requestCompletionMarker":true}))).await?;
                            pending.insert(id, Pending::Thread(thread_id));
                            next_id += 1;
                        }
                    }
                    Command::Call { tag, payload, reply } => {
                        if !synchronized {
                            let _=reply.send(Err("History is not synchronized; request was not sent".into())).await;
                            continue;
                        }
                        send(&mut socket, protocol::request(next_id, &tag, payload)).await?;
                        pending.insert(id, Pending::Call(reply));
                        next_id += 1;
                    }
                }
            }
            _ = heartbeat.tick() => {
                if !pong_received { return Err("Server heartbeat timed out"); }
                pong_received = false;
                send(&mut socket, ClientMessage::Ping).await?;
            }
            incoming = socket.next() => {
                let incoming = incoming.ok_or("Server disconnected")?.map_err(|_| "WebSocket transport failed")?;
                let bytes = match incoming {
                    Message::Text(text) => text.as_bytes().to_vec(),
                    Message::Binary(bytes) => bytes.to_vec(),
                    Message::Close(_) => return Err("Server disconnected"),
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                };
                let messages = protocol::decode_frame(&bytes).map_err(|_| "Unsupported or malformed RPC message")?;
                for message in messages {
                    match message {
                        ServerMessage::Pong => pong_received = true,
                        ServerMessage::Chunk { request_id, values } => {
                            if values.is_empty() { return Err("Empty RPC stream chunk"); }
                            if let Some(kind) = pending.get(&request_id) {
                                for value in values {
                                    let update = match kind {
                                        Pending::Shell => {
                                            let item:ShellItem=serde_json::from_value(value).map_err(|_| "Unsupported shell stream event")?;
                                            if matches!(item,ShellItem::Synchronized) { synchronized=true; *reached_sync=true; }
                                            Update::Shell(Box::new(item))
                                        },
                                        Pending::Thread(thread_id) => Update::Thread { thread_id: thread_id.clone(), item: value },
                                        Pending::Subscription(key) => Update::Subscription { key:key.clone(),item:Ok(value) },
                                        Pending::Call(_) => return Err("Unary RPC returned a stream chunk"),
                                    };
                                    sender.send(update).await.map_err(|_| "Client closed")?;
                                }
                            } else if !cancelled.contains(&request_id) { return Err("Unexpected RPC request ID"); }
                            // Acknowledge only once the bounded consumer has accepted the full chunk.
                            send(&mut socket, ClientMessage::Ack { request_id }).await?;
                        }
                        ServerMessage::Exit { request_id, exit } => {
                            if cancelled.remove(&request_id) { continue; }
                            let kind = pending.remove(&request_id).ok_or("Unexpected RPC request ID")?;
                            match kind {
                                Pending::Call(reply) => {
                                    let result = match exit { RpcExit::Success { value } => Ok(value), RpcExit::Failure { cause } => Err(rpc_failure(&cause)) };
                                    let _ = reply.send(result).await;
                                }
                                Pending::Subscription(key) => {
                                    let message = match exit { RpcExit::Success { .. } => "Read subscription ended".into(),RpcExit::Failure { cause } => rpc_failure(&cause) };
                                    sender.send(Update::Subscription { key,item:Err(message) }).await.map_err(|_| "Client closed")?;
                                }
                                Pending::Thread(_) => {
                                    let status = match exit { RpcExit::Success { .. } => "Thread subscription ended".to_owned(), RpcExit::Failure { cause } => rpc_failure(&cause) };
                                    sender.send(Update::Status(status)).await.map_err(|_| "Client closed")?;
                                }
                                Pending::Shell => return Err("Shell subscription ended or failed; reconnect to synchronize"),
                            }
                        }
                        ServerMessage::Defect { defect } => { drop(defect); return Err("Server reported an RPC defect"); }
                        ServerMessage::ClientProtocolError { error } => { drop(error); return Err("Server rejected the RPC protocol"); }
                    }
                }
            }
        }
    }
}

fn rpc_failure(cause: &Value) -> String {
    let error = cause.as_array().and_then(|entries| {
        entries
            .iter()
            .find_map(|entry| entry.get("error").filter(|error| error["_tag"].is_string()))
    });
    let name = error
        .and_then(|error| error["_tag"].as_str())
        .unwrap_or("RequestFailed");
    // Only the contract's user-facing fields belong in the UI. Commands, headers and
    // nested defects can contain credentials and must never be serialized as an error.
    let detail = error
        .and_then(|error| {
            error["message"]
                .as_str()
                .or_else(|| error["detail"].as_str())
        })
        .filter(|detail| !detail.trim().is_empty());
    match detail {
        Some(detail) => format!("{} ({name})", safe_error_detail(detail)),
        None => format!("Server rejected request ({name})"),
    }
}
fn safe_error_detail(detail: &str) -> String {
    let bounded: String = detail
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .take(4000)
        .collect();
    bounded
        .split_inclusive(char::is_whitespace)
        .map(|word| {
            let trimmed = word.trim();
            let core = trimmed.trim_matches(['\'', '"', '(', ')', ',']);
            if let Ok(mut url) = url::Url::parse(core)
                && matches!(url.scheme(), "http" | "https" | "ssh")
            {
                let _ = url.set_username("");
                let _ = url.set_password(None);
                url.set_query(None);
                url.set_fragment(None);
                word.replace(core, url.as_str())
            } else {
                word.to_owned()
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "reconnect_tests.rs"]
mod reconnect_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_configuration_rejects_credential_leaks_and_unsafe_origins() {
        for base in [
            "http://remote.example",
            "https://user:password@host",
            "https://host/?token=x",
            "https://host/#secret",
            "file:///tmp/x",
            "https://host/prefix/",
        ] {
            assert!(
                ConnectionOptions::new(base, "token".into()).is_err(),
                "{base}"
            );
        }
        for base in [
            "http://127.0.0.1:3777",
            "http://localhost:3777",
            "https://host",
        ] {
            assert!(
                ConnectionOptions::new(base, "token".into()).is_ok(),
                "{base}"
            );
        }
        assert!(ConnectionOptions::new("https://host", "".into()).is_err());
    }

    #[test]
    fn typed_server_failures_keep_actionable_reason_without_dumping_command_or_cause() {
        let error = rpc_failure(
            &json!([{"_tag":"Fail","error":{"_tag":"GitCommandError","detail":"Worktree contains modified or untracked files","command":"secret command","cause":{"token":"secret cause"}}}]),
        );
        assert!(error.contains("Worktree contains modified or untracked files"));
        assert!(!error.contains("secret"));
        assert!(rpc_failure(&json!([{"_tag":"Fail","error":{"_tag":"ProjectReadFileError","message":"File not found"}}])).contains("File not found"));
        assert_eq!(
            rpc_failure(&json!([{"_tag":"Die","defect":{"password":"hidden"}}])),
            "Server rejected request (RequestFailed)"
        );
    }

    #[test]
    fn actionable_errors_redact_url_credentials_and_query_tokens() {
        let result = rpc_failure(
            &json!([{"error":{"_tag":"SourceControlRepositoryError","detail":"Cannot clone https://user:password@example.invalid/repo?token=secret#private"}}]),
        );
        assert!(result.contains("Cannot clone https://example.invalid/repo"));
        assert!(!result.contains("password"));
        assert!(!result.contains("secret"));
        assert!(!result.contains("private"));
    }

    #[tokio::test]
    async fn rejected_authorization_reports_failure_without_leaking_credentials() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            stream
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let receiver = start(
            ConnectionOptions::new(&format!("http://{address}"), "private-secret".into()).unwrap(),
        );
        let mut failed = false;
        while let Ok(update) = receiver.recv().await {
            match update {
                Update::Status(status) => {
                    assert!(!status.contains("private-secret"));
                    failed |= status.contains("Authorization rejected");
                }
                Update::Resynchronizing => {}
                Update::Shell(_) | Update::Thread { .. } | Update::Subscription { .. } => {
                    panic!("unauthorized connection delivered shell state")
                }
            }
        }
        assert!(failed);
        server.await.unwrap();
    }

    #[tokio::test]
    // tungstenite's handshake callback fixes the HTTP response error type.
    #[allow(clippy::result_large_err)]
    async fn bearer_ticket_is_exchanged_before_authenticated_websocket() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut http, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                http.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(request.len() < 8192);
            }
            let request = String::from_utf8(request).unwrap().to_lowercase();
            assert!(request.starts_with("post /api/auth/websocket-ticket "));
            assert!(request.contains("authorization: bearer test-secret"));
            let body = r#"{"ticket":"one-time-ticket","expiresAt":"2026-01-01T00:00:00Z"}"#;
            http.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes()).await.unwrap();
            drop(http);
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    let uri = request.uri().to_string();
                    assert!(uri.starts_with("/ws?"));
                    assert!(uri.contains("wsTicket=one-time-ticket"));
                    assert!(uri.contains("orchestrationProtocol=1"));
                    assert!(uri.contains("clientSurface=desktop"));
                    assert!(uri.contains("clientOs=macOS"));
                    assert!(!uri.contains("test-secret"));
                    Ok(response)
                },
            )
            .await
            .unwrap();
            let _ = socket.next().await.unwrap().unwrap();
            socket.send(Message::Text(json!({"_tag":"Chunk","requestId":"1","values":[{"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[],"threads":[]}},{"kind":"synchronized"}]}).to_string().into())).await.unwrap();
            let ack = socket.next().await.unwrap().unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(ack.to_text().unwrap()).unwrap()["_tag"],
                "Ack"
            );
            socket.close(None).await.unwrap();
        });
        let receiver = start(
            ConnectionOptions::new(&format!("http://{address}"), "test-secret".into()).unwrap(),
        );
        let mut shell = crate::shell::Shell::default();
        while let Ok(update) = receiver.recv().await {
            if let Update::Shell(item) = update {
                shell.apply(*item).unwrap();
                if shell.synchronized {
                    break;
                }
            }
        }
        assert!(shell.synchronized);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn dropping_ui_receiver_cancels_pending_authorization() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted, request_started) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            accepted.send(()).unwrap();
            // The HTTP response is intentionally withheld. Cancellation must close it.
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
        });
        let receiver =
            start(ConnectionOptions::new(&format!("http://{address}"), "token".into()).unwrap());
        request_started.await.unwrap();
        drop(receiver);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn switching_threads_cancels_old_stream_and_correlates_unary_receipts() {
        async fn read(
            socket: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        ) -> Value {
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap()
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert_eq!(
                read(&mut socket).await["tag"],
                "orchestration.subscribeShell"
            );
            socket.send(Message::Text(json!({"_tag":"Chunk","requestId":"1","values":[{"kind":"snapshot","snapshot":{"snapshotSequence":0,"projects":[],"threads":[]}},{"kind":"synchronized"}]}).to_string().into())).await.unwrap();
            assert_eq!(read(&mut socket).await["_tag"], "Ack");
            let first = read(&mut socket).await;
            assert_eq!(first["payload"]["threadId"], "a");
            assert_eq!(first["id"], "2");
            socket
                .send(Message::Text(
                    json!({"_tag":"Chunk","requestId":"2","values":[{"kind":"synchronized"}]})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            assert_eq!(
                read(&mut socket).await,
                json!({"_tag":"Ack","requestId":"2"})
            );
            assert_eq!(
                read(&mut socket).await,
                json!({"_tag":"Interrupt","requestId":"2"})
            );
            assert_eq!(read(&mut socket).await["payload"]["threadId"], "b");
            socket
                .send(Message::Text(
                    json!([
                        {"_tag":"Chunk","requestId":"2","values":[{"kind":"stale-a"}]},
                        {"_tag":"Exit","requestId":"2","exit":{"_tag":"Failure","cause":[]}},
                        {"_tag":"Chunk","requestId":"3","values":[{"kind":"synchronized"}]}
                    ])
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            assert_eq!(
                read(&mut socket).await,
                json!({"_tag":"Ack","requestId":"2"})
            );
            assert_eq!(
                read(&mut socket).await,
                json!({"_tag":"Ack","requestId":"3"})
            );
            let unary = read(&mut socket).await;
            assert_eq!(unary["tag"], "orchestration.dispatchCommand");
            assert_eq!(unary["id"], "4");
            socket.send(Message::Text(json!({"_tag":"Exit","requestId":"4","exit":{"_tag":"Success","value":{"sequence":9}}}).to_string().into())).await.unwrap();
            socket.close(None).await.unwrap();
        });
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (sender, receiver) = async_channel::bounded(8);
        let (commands, requests) = async_channel::bounded(8);
        let driver = tokio::spawn(async move { run_session(socket, &sender, &requests).await });
        for _ in 0..2 {
            assert!(matches!(receiver.recv().await.unwrap(), Update::Shell(_)));
        }
        commands
            .send(Command::SelectThread(Some("a".into())))
            .await
            .unwrap();
        assert!(
            matches!(receiver.recv().await.unwrap(), Update::Thread { thread_id, .. } if thread_id == "a")
        );
        commands
            .send(Command::SelectThread(Some("b".into())))
            .await
            .unwrap();
        assert!(
            matches!(receiver.recv().await.unwrap(), Update::Thread { thread_id, item } if thread_id == "b" && item["kind"] == "synchronized")
        );
        assert_eq!(
            call(&commands, "orchestration.dispatchCommand", json!({}))
                .await
                .unwrap(),
            json!({"sequence":9})
        );
        assert!(receiver.try_recv().is_err());
        assert!(driver.await.unwrap().is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn socket_subscription_delivers_snapshot_and_acks_chunk() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let request: serde_json::Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(request["tag"], "orchestration.subscribeShell");
            assert_eq!(request["payload"], json!({"requestCompletionMarker":true}));
            socket.send(Message::Text(json!({"_tag":"Chunk","requestId":"1","values":[{"kind":"snapshot","snapshot":{"snapshotSequence":2,"projects":[{"id":"p","title":"Example","workspaceRoot":"/tmp/example"}],"threads":[]}},{"kind":"synchronized"}]}).to_string().into())).await.unwrap();
            let ack: serde_json::Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(ack, json!({"_tag":"Ack","requestId":"1"}));
            socket.close(None).await.unwrap();
        });
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let (sender, receiver) = async_channel::bounded(4);
        let result = run_socket(socket, &sender).await;
        assert!(result.is_err());
        let mut shell = crate::shell::Shell::default();
        for _ in 0..2 {
            match receiver.recv().await.unwrap() {
                Update::Shell(item) => shell.apply(*item).unwrap(),
                _ => panic!("expected shell item"),
            }
        }
        assert_eq!(shell.projects[0].title, "Example");
        assert!(shell.synchronized);
        server.await.unwrap();
    }
}

#[cfg(test)]
mod subscription_tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::Role;
    async fn receive(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio::io::DuplexStream>,
    ) -> Value {
        let message = socket.next().await.unwrap().unwrap();
        serde_json::from_str(message.to_text().unwrap()).unwrap()
    }
    #[tokio::test]
    async fn read_subscription_replacement_cancels_old_stream_and_rejects_mutation_tags() {
        let (client, server) = tokio::io::duplex(16384);
        let client =
            tokio_tungstenite::WebSocketStream::from_raw_socket(client, Role::Client, None).await;
        let mut server =
            tokio_tungstenite::WebSocketStream::from_raw_socket(server, Role::Server, None).await;
        let (updates, rx) = async_channel::bounded(16);
        let (commands, requests) = async_channel::bounded(16);
        let task = tokio::spawn(async move { run_session(client, &updates, &requests).await });
        assert_eq!(
            receive(&mut server).await["tag"],
            "orchestration.subscribeShell"
        );
        commands
            .send(Command::Subscribe {
                key: "clones".into(),
                tag: "subscribeProjectClones".into(),
                payload: json!({}),
            })
            .await
            .unwrap();
        let old = receive(&mut server).await;
        commands
            .send(Command::Subscribe {
                key: "clones".into(),
                tag: "subscribeProjectClones".into(),
                payload: json!({}),
            })
            .await
            .unwrap();
        let interrupt = receive(&mut server).await;
        assert_eq!(interrupt["_tag"], "Interrupt");
        assert_eq!(interrupt["requestId"], old["id"]);
        let fresh = receive(&mut server).await;
        server
            .send(Message::Text(
                json!({"_tag":"Chunk","requestId":old["id"],"values":[{"stale":true}]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        assert_eq!(receive(&mut server).await["_tag"], "Ack");
        server
            .send(Message::Text(
                json!({"_tag":"Chunk","requestId":fresh["id"],"values":[[]]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        assert!(
            matches!(rx.recv().await.unwrap(),Update::Subscription { key,item:Ok(value) } if key == "clones" && value == json!([]))
        );
        assert_eq!(receive(&mut server).await["_tag"], "Ack");
        commands
            .send(Command::Subscribe {
                key: "unsafe".into(),
                tag: "orchestration.dispatchCommand".into(),
                payload: json!({}),
            })
            .await
            .unwrap();
        assert!(
            matches!(rx.recv().await.unwrap(),Update::Subscription { key,item:Err(_) } if key == "unsafe")
        );
        commands
            .send(Command::Unsubscribe("clones".into()))
            .await
            .unwrap();
        assert_eq!(receive(&mut server).await["requestId"], fresh["id"]);
        server.close(None).await.unwrap();
        assert_eq!(task.await.unwrap(), Err("Server disconnected"));
    }
    #[tokio::test]
    async fn disconnected_read_subscriptions_are_restored_but_mutations_are_not_retained() {
        let mut selected = None;
        let mut subscriptions = Subscriptions::new();
        reject_disconnected(
            Command::Subscribe {
                key: "clones".into(),
                tag: "subscribeProjectClones".into(),
                payload: json!({}),
            },
            &mut selected,
            &mut subscriptions,
        )
        .await;
        reject_disconnected(
            Command::Subscribe {
                key: "unsafe".into(),
                tag: "orchestration.dispatchCommand".into(),
                payload: json!({}),
            },
            &mut selected,
            &mut subscriptions,
        )
        .await;
        assert_eq!(subscriptions.len(), 1);
        reject_disconnected(
            Command::Unsubscribe("clones".into()),
            &mut selected,
            &mut subscriptions,
        )
        .await;
        assert!(subscriptions.is_empty());
    }
}
