//! Blocking client.
//!
//! `lumina-send`, integration tests, and the Python and Node clients all speak
//! the same protocol. This client keeps the implementation small: one request
//! at a time, events buffered in arrival order while waiting for the matching
//! response.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, BufReader, Write};

use serde_json::Value;

use crate::events::Event;
use crate::framing::{FrameError, LineReader};
use crate::protocol::{Command, IpcError, Request, RequestId, Response};
use crate::transport::{self, Connection, IpcAddress};

const MAX_BUFFERED_EVENTS: usize = 256;

/// A client-side failure.
#[derive(Debug)]
pub enum ClientError {
    /// The connection failed or closed.
    Io(io::Error),
    /// A framing violation occurred.
    Frame(FrameError),
    /// The server sent something that did not match the request.
    Protocol(String),
    /// The command was rejected by the server.
    Remote(IpcError),
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "connection failed: {error}"),
            Self::Frame(error) => write!(formatter, "{error}"),
            Self::Protocol(message) => write!(formatter, "protocol error: {message}"),
            Self::Remote(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Frame(error) => Some(error),
            Self::Remote(error) => Some(error),
            Self::Protocol(_) => None,
        }
    }
}

impl From<io::Error> for ClientError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FrameError> for ClientError {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

enum Incoming {
    Response(Response),
    Event(Event),
}

fn parse_incoming(line: &str) -> Result<Incoming, ClientError> {
    let value: Value = serde_json::from_str(line)
        .map_err(|error| ClientError::Protocol(format!("invalid JSON: {error}")))?;
    if value.get("event").is_some() {
        serde_json::from_value(value)
            .map(Incoming::Event)
            .map_err(|error| ClientError::Protocol(format!("invalid event: {error}")))
    } else {
        serde_json::from_value(value)
            .map(Incoming::Response)
            .map_err(|error| ClientError::Protocol(format!("invalid response: {error}")))
    }
}

/// A blocking IPC client.
pub struct Client {
    reader: LineReader<BufReader<Box<dyn Connection>>>,
    writer: Box<dyn Connection>,
    next_id: i64,
    events: VecDeque<Event>,
}

impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Client")
            .field("next_id", &self.next_id)
            .field("buffered_events", &self.events.len())
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Connect to a running Lumina instance.
    pub fn connect(address: &IpcAddress) -> io::Result<Self> {
        let writer = transport::connect(address)?;
        let reader = writer.try_clone_box()?;
        Ok(Self {
            reader: LineReader::new(BufReader::new(reader)),
            writer,
            next_id: 0,
            events: VecDeque::new(),
        })
    }

    /// Send a command and wait for its response.
    ///
    /// Events that arrive before the response are buffered and returned by
    /// [`Client::next_event`].
    pub fn call(&mut self, command: &Command) -> Result<Value, ClientError> {
        self.next_id += 1;
        self.call_with_id(RequestId::Number(self.next_id), command)
    }

    /// Send a command with an explicit id.
    pub fn call_with_id(&mut self, id: RequestId, command: &Command) -> Result<Value, ClientError> {
        let request = Request {
            id: Some(id),
            command: command.clone(),
        };
        let expected = request.id.clone();
        let line = request
            .to_line()
            .map_err(|error| ClientError::Protocol(format!("could not encode request: {error}")))?;
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()?;

        loop {
            let Some(line) = self.reader.read_line()? else {
                return Err(ClientError::Protocol(
                    "server closed the connection before responding".to_string(),
                ));
            };
            match parse_incoming(&line)? {
                Incoming::Event(event) => self.push_event(event),
                Incoming::Response(response) => {
                    if response
                        .id
                        .as_ref()
                        .is_some_and(|received| expected.as_ref() != Some(received))
                    {
                        let received = response
                            .id
                            .as_ref()
                            .map_or_else(|| "null".to_string(), ToString::to_string);
                        let expected = expected
                            .as_ref()
                            .map_or_else(|| "null".to_string(), ToString::to_string);
                        return Err(ClientError::Protocol(format!(
                            "response id {received} does not match request id {expected}"
                        )));
                    }
                    if response.ok {
                        return Ok(response.data.unwrap_or(Value::Null));
                    }
                    let error = response.error.unwrap_or_else(|| {
                        IpcError::internal("server returned an error without a body")
                    });
                    return Err(ClientError::Remote(error));
                }
            }
        }
    }

    /// Take the next buffered event, without reading from the socket.
    pub fn next_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Read one event from the socket, blocking until one arrives.
    ///
    /// Use this from a dedicated thread when subscribing to
    /// `state_changed` or `media_changed`. Responses are not expected on this
    /// path; receiving one is a protocol error.
    pub fn read_event(&mut self) -> Result<Event, ClientError> {
        let Some(line) = self.reader.read_line()? else {
            return Err(ClientError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "server closed the connection",
            )));
        };
        match parse_incoming(&line)? {
            Incoming::Event(event) => Ok(event),
            Incoming::Response(_) => Err(ClientError::Protocol(
                "received an unexpected response while waiting for an event".to_string(),
            )),
        }
    }

    fn push_event(&mut self, event: Event) {
        if self.events.len() >= MAX_BUFFERED_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incoming_responses_and_events_are_distinguished() {
        match parse_incoming(r#"{"id":1,"ok":true,"data":"ok"}"#).unwrap() {
            Incoming::Response(response) => assert!(response.ok),
            Incoming::Event(_) => panic!("expected a response"),
        }
        match parse_incoming(r#"{"event":"state_changed","data":{"state":"hover"}}"#).unwrap() {
            Incoming::Event(event) => assert_eq!(event.event.as_str(), "state_changed"),
            Incoming::Response(_) => panic!("expected an event"),
        }
    }

    #[test]
    fn invalid_lines_are_protocol_errors() {
        assert!(matches!(
            parse_incoming("{nope"),
            Err(ClientError::Protocol(_))
        ));
    }

    #[test]
    fn remote_errors_render_the_code_and_message() {
        let error = ClientError::Remote(IpcError::unknown_command("dance"));
        assert!(error.to_string().contains("UnknownCommand"));
        assert!(error.to_string().contains("dance"));
    }
}
