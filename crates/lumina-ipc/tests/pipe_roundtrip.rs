use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lumina_ipc::{
    transport, Client, Command, EventSender, Handler, IpcAddress, IpcError, Server, ServerOptions,
};
use serde_json::{json, Value};

struct Echo;

impl Handler for Echo {
    fn handle(&mut self, _command: Command) -> Result<Value, IpcError> {
        Ok(json!({"state": "collapsed"}))
    }
}

#[test]
fn a_pipe_server_answers_a_client() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let address = IpcAddress::pipe(format!(
        r"\\.\pipe\lumina-test-roundtrip-{}-{stamp}",
        std::process::id()
    ));
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
        .expect("the server did not answer a pipe client");
    assert!(answered);

    server.shutdown();
    server.wait().unwrap();
}
