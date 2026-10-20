use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lumina_ipc::{
    transport, Client, Command, Event, EventKind, EventSender, Handler, IpcAddress, IpcError,
    Server, ServerOptions,
};
use serde_json::{json, Value};

struct Echo;

impl Handler for Echo {
    fn handle(&mut self, _command: Command) -> Result<Value, IpcError> {
        Ok(json!({"state": "collapsed"}))
    }
}

fn unique_address(tag: &str) -> IpcAddress {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let name = format!("lumina-test-{}-{stamp}-{tag}", std::process::id());
    #[cfg(unix)]
    {
        IpcAddress::unix(std::env::temp_dir().join(format!("{name}.sock")))
    }
    #[cfg(windows)]
    {
        IpcAddress::pipe(format!(r"\\.\pipe\{name}"))
    }
}

#[test]
fn a_server_answers_a_client() {
    let address = unique_address("request");
    let listener = transport::bind(&address).unwrap();
    let server = Server::new(listener, Echo, ServerOptions::default())
        .spawn(&EventSender::default())
        .unwrap();

    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut client = Client::connect(&address).unwrap();
        let result = client.call(&Command::Status).is_ok();
        let _ = sender.send(result);
    });

    let answered = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("the server did not answer a client");
    assert!(answered);

    server.shutdown();
    server.wait().unwrap();
}

#[test]
fn a_client_receives_pushed_events() {
    let address = unique_address("events");
    let events = EventSender::default();
    let listener = transport::bind(&address).unwrap();
    let server = Server::new(listener, Echo, ServerOptions::default())
        .spawn(&events)
        .unwrap();

    let bus = events.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut client = Client::connect(&address).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while bus.subscriber_count() == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = sender.send(client.read_event());
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    while events.subscriber_count() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(events.subscriber_count() > 0, "the client never subscribed");
    events.emit(&Event::state_changed("hover")).unwrap();

    let event = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("the server did not push the event")
        .expect("the event frame was invalid");
    assert_eq!(event.event, EventKind::StateChanged);

    server.shutdown();
    server.wait().unwrap();
}
