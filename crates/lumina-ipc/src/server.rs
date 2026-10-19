//! Multi-client server.
//!
//! One thread accepts connections, one thread runs the handler, and each
//! connection gets a reader plus a writer so events can be pushed while a
//! request is in flight. Handler calls are serialized, which makes command
//! ordering deterministic even when several scripts talk to Lumina at once.

use std::fmt;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use crate::events::EventSender;
use crate::framing::{encode_line, FrameError, LineReader};
use crate::protocol::{Command, IpcError, Request, Response, MAX_CLIENTS};
use crate::rate_limit::{RateLimiter, DEFAULT_RATE_PER_SECOND};
use crate::transport::{Connection, IpcAddress, Listener};

/// Default number of milliseconds between accept attempts.
const DEFAULT_POLL_MS: u64 = 10;

/// How often a connection thread checks for shutdown while waiting on events.
const EVENT_POLL: Duration = Duration::from_millis(50);

/// Tunables for a [`Server`].
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// Concurrent connection limit. Clamped to `1..=MAX_CLIENTS`.
    pub max_clients: usize,
    /// Per-client command budget per second.
    pub rate_per_second: u32,
    /// Idle time between non-blocking accept attempts.
    pub poll_interval: Duration,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            max_clients: MAX_CLIENTS,
            rate_per_second: DEFAULT_RATE_PER_SECOND,
            poll_interval: Duration::from_millis(DEFAULT_POLL_MS),
        }
    }
}

/// The application side of the IPC surface.
///
/// Calls run on a single dispatcher thread, so implementations do not need to
/// be thread safe, but they must not block indefinitely: the whole control
/// surface waits for the current command.
pub trait Handler: Send {
    /// Execute one command.
    fn handle(&mut self, command: Command) -> Result<Value, IpcError>;
}

struct Job {
    command: Command,
    reply: SyncSender<Result<Value, IpcError>>,
}

/// A bound server.
pub struct Server {
    listener: Box<dyn Listener>,
    handler: Box<dyn Handler>,
    options: ServerOptions,
}

impl fmt::Debug for Server {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Server")
            .field("address", &self.listener.address())
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Bind a server to an already created listener.
    pub fn new(
        listener: Box<dyn Listener>,
        handler: impl Handler + 'static,
        options: ServerOptions,
    ) -> Self {
        Self {
            listener,
            handler: Box::new(handler),
            options,
        }
    }

    /// The address the server listens on.
    pub fn address(&self) -> &IpcAddress {
        self.listener.address()
    }

    /// Run until `quit` succeeds or the handle is shut down. Blocks the caller.
    pub fn serve(self, events: &EventSender) -> io::Result<()> {
        let shutdown = Arc::new(AtomicBool::new(false));
        self.serve_inner(events, &shutdown)
    }

    /// Run the server on a background thread.
    pub fn spawn(self, events: &EventSender) -> io::Result<ServerHandle> {
        let address = self.address().clone();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let thread_events = events.clone();
        let join = thread::Builder::new()
            .name("lumina-ipc-accept".to_string())
            .spawn(move || self.serve_inner(&thread_events, &thread_shutdown))?;
        Ok(ServerHandle {
            address,
            shutdown,
            events: events.clone(),
            join: Some(join),
        })
    }

    fn serve_inner(self, events: &EventSender, shutdown: &Arc<AtomicBool>) -> io::Result<()> {
        let Self {
            mut listener,
            handler,
            options,
        } = self;
        let poll = options.poll_interval.max(Duration::from_millis(1));
        let max_clients = options.max_clients.clamp(1, MAX_CLIENTS);
        let rate = options.rate_per_second;

        let (job_sender, job_receiver) = mpsc::channel::<Job>();
        let dispatcher = thread::Builder::new()
            .name("lumina-ipc-dispatch".to_string())
            .spawn({
                let shutdown = Arc::clone(shutdown);
                move || dispatch_loop(handler, &job_receiver, &shutdown, poll)
            })?;

        let clients = Arc::new(AtomicUsize::new(0));
        while !shutdown.load(Ordering::Acquire) {
            match listener.accept() {
                Ok(Some(connection)) => {
                    if clients.load(Ordering::Relaxed) >= max_clients {
                        reject_busy(connection, max_clients);
                        continue;
                    }
                    clients.fetch_add(1, Ordering::Relaxed);
                    let client_jobs = job_sender.clone();
                    let client_events = events.clone();
                    let client_shutdown = Arc::clone(shutdown);
                    let counter = Arc::clone(&clients);
                    let spawned = thread::Builder::new()
                        .name("lumina-ipc-client".to_string())
                        .spawn(move || {
                            run_connection(
                                connection,
                                &client_jobs,
                                &client_events,
                                &client_shutdown,
                                rate,
                            );
                            counter.fetch_sub(1, Ordering::Relaxed);
                        });
                    if spawned.is_err() {
                        clients.fetch_sub(1, Ordering::Relaxed);
                        return Err(io::Error::other("could not spawn an IPC client thread"));
                    }
                }
                Ok(None) => thread::sleep(poll),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    drop(job_sender);
                    let _ = dispatcher.join();
                    return Err(error);
                }
            }
        }

        drop(job_sender);
        let _ = dispatcher.join();
        Ok(())
    }
}

fn dispatch_loop(
    mut handler: Box<dyn Handler>,
    jobs: &Receiver<Job>,
    shutdown: &AtomicBool,
    poll: Duration,
) {
    loop {
        match jobs.recv_timeout(poll) {
            Ok(job) => {
                let quit = matches!(job.command, Command::Quit);
                let result = handler.handle(job.command);
                let succeeded = result.is_ok();
                let _ = job.reply.send(result);
                if quit && succeeded {
                    shutdown.store(true, Ordering::Release);
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn reject_busy(mut connection: Box<dyn Connection>, max_clients: usize) {
    let error = IpcError::rate_limited();
    let response = Response::failure(
        None,
        IpcError::new(
            error.code,
            format!("maximum of {max_clients} clients reached"),
        ),
    );
    if let Ok(line) = encode_line(&response) {
        let _ = connection.write_all(&line);
        let _ = connection.flush();
    }
}

fn run_connection(
    connection: Box<dyn Connection>,
    jobs: &Sender<Job>,
    events: &EventSender,
    shutdown: &AtomicBool,
    rate: u32,
) {
    let Ok(writer) = connection.try_clone_box() else {
        return;
    };
    let (out_sender, out_receiver) = mpsc::channel::<Vec<u8>>();

    let writer_thread = thread::spawn(move || {
        let mut writer = writer;
        for line in out_receiver {
            if writer
                .write_all(&line)
                .and_then(|()| writer.flush())
                .is_err()
            {
                break;
            }
        }
    });

    let subscription = events.subscribe();
    let closed = Arc::new(AtomicBool::new(false));
    let forward_closed = Arc::clone(&closed);
    let forward_sender = out_sender.clone();
    let forwarder = thread::spawn(move || loop {
        match subscription.recv_timeout(EVENT_POLL) {
            Ok(line) => {
                if forward_sender.send(line).is_err() {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if forward_closed.load(Ordering::Acquire) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    });

    let mut reader = LineReader::new(io::BufReader::new(connection));
    let mut limiter = RateLimiter::new(rate);
    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let line = match reader.read_line() {
            Ok(Some(line)) => line,
            Ok(None) | Err(FrameError::Io(_)) => break,
            Err(FrameError::TooLong) => {
                send(
                    &out_sender,
                    &Response::failure(None, IpcError::line_too_long()),
                );
                break;
            }
            Err(FrameError::InvalidUtf8) => {
                send(
                    &out_sender,
                    &Response::failure(None, IpcError::bad_request("line is not valid UTF-8")),
                );
                continue;
            }
        };
        if line.trim().is_empty() {
            send(
                &out_sender,
                &Response::failure(None, IpcError::bad_request("empty request line")),
            );
            continue;
        }
        let request = match Request::parse(&line) {
            Ok(request) => request,
            Err(error) => {
                send(&out_sender, &Response::failure(None, error));
                continue;
            }
        };
        if !limiter.try_acquire() {
            send(
                &out_sender,
                &Response::failure(request.id, IpcError::rate_limited()),
            );
            continue;
        }
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        let job = Job {
            command: request.command,
            reply: reply_sender,
        };
        if jobs.send(job).is_err() {
            send(
                &out_sender,
                &Response::failure(request.id, IpcError::internal("server is shutting down")),
            );
            break;
        }
        match reply_receiver.recv() {
            Ok(Ok(data)) => send(&out_sender, &Response::success(request.id, data)),
            Ok(Err(error)) => send(&out_sender, &Response::failure(request.id, error)),
            Err(_) => {
                send(
                    &out_sender,
                    &Response::failure(
                        request.id,
                        IpcError::internal("command handler did not respond"),
                    ),
                );
                break;
            }
        }
    }

    closed.store(true, Ordering::Release);
    drop(out_sender);
    let _ = forwarder.join();
    let _ = writer_thread.join();
}

fn send(out: &Sender<Vec<u8>>, response: &Response) {
    if let Ok(line) = encode_line(response) {
        let _ = out.send(line);
    }
}

/// A running server on a background thread.
pub struct ServerHandle {
    address: IpcAddress,
    shutdown: Arc<AtomicBool>,
    events: EventSender,
    join: Option<JoinHandle<io::Result<()>>>,
}

impl fmt::Debug for ServerHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerHandle")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl ServerHandle {
    /// The address clients should connect to.
    pub fn address(&self) -> &IpcAddress {
        &self.address
    }

    /// The event bus wired into this server.
    pub fn events(&self) -> &EventSender {
        &self.events
    }

    /// Ask the accept loop to stop. In-flight requests may still complete.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    /// Wait for the accept loop to finish.
    pub fn wait(mut self) -> io::Result<()> {
        self.shutdown();
        match self.join.take() {
            Some(join) => join
                .join()
                .map_err(|_| io::Error::other("IPC accept thread panicked"))?,
            None => Ok(()),
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Event;
    use std::io::{BufRead, BufReader, Read};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc as StdArc, Mutex};
    use std::time::Instant;

    #[derive(Debug)]
    struct TestConnection(TcpStream);

    impl Read for TestConnection {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.0.read(buffer)
        }
    }

    impl Write for TestConnection {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.write(buffer)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    impl Connection for TestConnection {
        fn try_clone_box(&self) -> io::Result<Box<dyn Connection>> {
            Ok(Box::new(TestConnection(self.0.try_clone()?)))
        }
    }

    #[derive(Debug)]
    struct TestListener {
        listener: TcpListener,
        address: IpcAddress,
    }

    impl Listener for TestListener {
        fn accept(&mut self) -> io::Result<Option<Box<dyn Connection>>> {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false)?;
                    Ok(Some(Box::new(TestConnection(stream))))
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
                Err(error) => Err(error),
            }
        }

        fn address(&self) -> &IpcAddress {
            &self.address
        }
    }

    fn test_address() -> IpcAddress {
        #[cfg(unix)]
        {
            IpcAddress::unix("/tmp/lumina-ipc-server-test.sock")
        }
        #[cfg(windows)]
        {
            IpcAddress::pipe(r"\\.\pipe\lumina-ipc-server-test")
        }
    }

    fn start<H: Handler + 'static>(handler: H, options: ServerOptions) -> (u16, ServerHandle) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let server = Server::new(
            Box::new(TestListener {
                listener,
                address: test_address(),
            }),
            handler,
            options,
        );
        let events = EventSender::new();
        let handle = server.spawn(&events).unwrap();
        (port, handle)
    }

    struct Echo;

    impl Handler for Echo {
        fn handle(&mut self, command: Command) -> Result<Value, IpcError> {
            match command {
                Command::Status => Ok(serde_json::json!({"state":"collapsed"})),
                Command::Hide | Command::Quit => Ok(Value::String("ok".to_string())),
                other => Ok(Value::String(other.name().to_string())),
            }
        }
    }

    struct Record(StdArc<Mutex<Vec<String>>>);

    impl Handler for Record {
        fn handle(&mut self, command: Command) -> Result<Value, IpcError> {
            self.0.lock().unwrap().push(command.name().to_string());
            Ok(Value::String("ok".to_string()))
        }
    }

    fn client(port: u16) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
    }

    fn send(stream: &mut TcpStream, line: &str) {
        stream.write_all(line.as_bytes()).unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
    }

    fn receive(reader: &mut BufReader<TcpStream>) -> Value {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "server closed the connection");
        serde_json::from_str(&line).unwrap()
    }

    fn pair(port: u16) -> (TcpStream, BufReader<TcpStream>) {
        let stream = client(port);
        let reader = BufReader::new(stream.try_clone().unwrap());
        (stream, reader)
    }

    #[test]
    fn requests_are_answered_with_the_matching_id() {
        let (port, handle) = start(Echo, ServerOptions::default());
        let (mut stream, mut reader) = pair(port);
        send(&mut stream, r#"{"id":1,"command":"status"}"#);
        send(&mut stream, r#"{"id":"two","command":"hide"}"#);
        let first = receive(&mut reader);
        let second = receive(&mut reader);
        assert_eq!(first["id"], 1);
        assert_eq!(first["ok"], true);
        assert_eq!(first["data"]["state"], "collapsed");
        assert_eq!(second["id"], "two");
        assert_eq!(second["data"], "ok");
        drop(stream);
        handle.wait().unwrap();
    }

    #[test]
    fn an_invalid_command_keeps_the_connection_open() {
        let (port, handle) = start(Echo, ServerOptions::default());
        let (mut stream, mut reader) = pair(port);
        send(&mut stream, "{not json");
        send(&mut stream, r#"{"id":2,"command":"dance"}"#);
        send(&mut stream, r#"{"id":3,"command":"hide"}"#);
        let bad_json = receive(&mut reader);
        let unknown = receive(&mut reader);
        let good = receive(&mut reader);
        assert_eq!(bad_json["error"]["code"], "BadRequest");
        assert_eq!(unknown["error"]["code"], "UnknownCommand");
        assert_eq!(good["ok"], true);
        drop(stream);
        handle.wait().unwrap();
    }

    #[test]
    fn an_overlong_line_is_answered_and_closes_the_connection() {
        let (port, handle) = start(Echo, ServerOptions::default());
        let (mut stream, mut reader) = pair(port);
        let line = "x".repeat(crate::MAX_LINE_BYTES + 1);
        send(&mut stream, &line);
        let response = receive(&mut reader);
        assert_eq!(response["error"]["code"], "LineTooLong");
        let mut rest = String::new();
        let read = reader.read_to_string(&mut rest).unwrap();
        assert_eq!(read, 0, "server closed the connection");
        drop(stream);
        handle.wait().unwrap();
    }

    #[test]
    fn the_rate_limit_keeps_the_connection_open() {
        let (port, handle) = start(
            Echo,
            ServerOptions {
                rate_per_second: 2,
                ..ServerOptions::default()
            },
        );
        let (mut stream, mut reader) = pair(port);
        send(&mut stream, r#"{"id":1,"command":"hide"}"#);
        send(&mut stream, r#"{"id":2,"command":"hide"}"#);
        send(&mut stream, r#"{"id":3,"command":"hide"}"#);
        assert_eq!(receive(&mut reader)["ok"], true);
        assert_eq!(receive(&mut reader)["ok"], true);
        assert_eq!(receive(&mut reader)["error"]["code"], "RateLimited");
        std::thread::sleep(Duration::from_millis(600));
        send(&mut stream, r#"{"id":4,"command":"hide"}"#);
        assert_eq!(receive(&mut reader)["ok"], true);
        drop(stream);
        handle.wait().unwrap();
    }

    #[test]
    fn events_are_pushed_to_connected_clients() {
        let (port, handle) = start(Echo, ServerOptions::default());
        let (_stream, mut reader) = pair(port);
        let deadline = Instant::now() + Duration::from_secs(2);
        while handle.events().subscriber_count() == 0 {
            assert!(Instant::now() < deadline, "client never subscribed");
            std::thread::sleep(Duration::from_millis(5));
        }
        handle
            .events()
            .emit(&Event::state_changed("expanded"))
            .unwrap();
        let event = receive(&mut reader);
        assert_eq!(event["event"], "state_changed");
        assert_eq!(event["data"]["state"], "expanded");
        handle.wait().unwrap();
    }

    #[test]
    fn commands_from_two_clients_are_serialized() {
        let order = StdArc::new(Mutex::new(Vec::new()));
        let (port, handle) = start(Record(StdArc::clone(&order)), ServerOptions::default());
        let (mut first, mut first_reader) = pair(port);
        let (mut second, mut second_reader) = pair(port);
        send(&mut first, r#"{"id":1,"command":"hide"}"#);
        send(&mut second, r#"{"id":2,"command":"collapse"}"#);
        assert_eq!(receive(&mut first_reader)["ok"], true);
        assert_eq!(receive(&mut second_reader)["ok"], true);
        let order = order.lock().unwrap();
        assert_eq!(order.len(), 2);
        assert!(order.contains(&"hide".to_string()));
        assert!(order.contains(&"collapse".to_string()));
        drop(first);
        drop(second);
        handle.wait().unwrap();
    }

    #[test]
    fn quitting_stops_the_server() {
        let (port, handle) = start(Echo, ServerOptions::default());
        let (mut stream, mut reader) = pair(port);
        send(&mut stream, r#"{"id":1,"command":"quit"}"#);
        assert_eq!(receive(&mut reader)["ok"], true);
        drop(stream);
        handle.wait().unwrap();
    }

    #[test]
    fn the_connection_limit_is_enforced() {
        let (port, handle) = start(
            Echo,
            ServerOptions {
                max_clients: 1,
                ..ServerOptions::default()
            },
        );
        let (mut first, mut first_reader) = pair(port);
        std::thread::sleep(Duration::from_millis(30));
        let (_second, mut second_reader) = pair(port);
        let rejected = receive(&mut second_reader);
        assert_eq!(rejected["error"]["code"], "RateLimited");
        send(&mut first, r#"{"id":1,"command":"hide"}"#);
        assert_eq!(receive(&mut first_reader)["ok"], true);
        drop(first);
        handle.wait().unwrap();
    }

    #[test]
    fn a_busy_client_does_not_block_the_process() {
        let (port, handle) = start(Echo, ServerOptions::default());
        let (_idle, _reader) = pair(port);
        let (mut active, mut active_reader) = pair(port);
        let start = Instant::now();
        send(&mut active, r#"{"id":1,"command":"status"}"#);
        assert_eq!(receive(&mut active_reader)["ok"], true);
        assert!(start.elapsed() < Duration::from_secs(1));
        drop(active);
        handle.wait().unwrap();
    }
}
