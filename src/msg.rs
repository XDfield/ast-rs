use std::{
    fmt,
    io::{self, BufRead, Write},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::error::ExtractError;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

impl From<Request> for Message {
    fn from(request: Request) -> Message {
        Message::Request(request)
    }
}

impl From<Response> for Message {
    fn from(response: Response) -> Message {
        Message::Response(response)
    }
}

impl From<Notification> for Message {
    fn from(notification: Notification) -> Message {
        Message::Notification(notification)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct RequestId(IdRepr);

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(untagged)]
enum IdRepr {
    I32(i32),
    String(String),
}

impl From<i32> for RequestId {
    fn from(id: i32) -> RequestId {
        RequestId(IdRepr::I32(id))
    }
}

impl From<String> for RequestId {
    fn from(id: String) -> RequestId {
        RequestId(IdRepr::String(id))
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            IdRepr::I32(it) => fmt::Display::fmt(it, f),
            // Use debug here, to make it clear that `92` and `"92"` are
            // different, and to reduce WTF factor if the sever uses `" "` as an
            // ID.
            IdRepr::String(it) => fmt::Debug::fmt(it, f),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub id: RequestId,
    pub method: String,
    #[serde(default = "serde_json::Value::default")]
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Response {
    // JSON RPC allows this to be null if it was impossible
    // to decode the request's id. Ignore this special case
    // and just die horribly.
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Notification {
    pub method: String,
    #[serde(default = "serde_json::Value::default")]
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
}

impl Message {
    pub fn read(r: &mut impl BufRead) -> io::Result<Option<Message>> {
        Message::_read(r)
    }
    fn _read(r: &mut dyn BufRead) -> io::Result<Option<Message>> {
        let text = match read_msg_text(r)? {
            None => return Ok(None),
            Some(text) => text,
        };
        let msg = serde_json::from_str(&text)?;
        Ok(Some(msg))
    }
    pub fn write(self, w: &mut impl Write) -> io::Result<()> {
        self._write(w)
    }
    fn _write(self, w: &mut dyn Write) -> io::Result<()> {
        #[derive(Serialize)]
        struct JsonRpc {
            jsonrpc: &'static str,
            #[serde(flatten)]
            msg: Message,
        }
        let text = serde_json::to_string(&JsonRpc { jsonrpc: "2.0", msg: self })?;
        write_msg_text(w, &text)
    }
}

impl Response {
    pub fn new_ok<R: Serialize>(id: RequestId, result: R) -> Response {
        Response { id, result: Some(serde_json::to_value(result).unwrap()), error: None }
    }
    pub fn new_err(id: RequestId, code: i32, message: String) -> Response {
        let error = ResponseError { code, message, data: None };
        Response { id, result: None, error: Some(error) }
    }
}

impl Request {
    pub fn new<P: Serialize>(id: RequestId, method: String, params: P) -> Request {
        Request { id, method, params: serde_json::to_value(params).unwrap() }
    }
    pub fn extract<P: DeserializeOwned>(
        self,
        method: &str,
    ) -> Result<(RequestId, P), ExtractError<Request>> {
        if self.method != method {
            return Err(ExtractError::MethodMismatch(self));
        }
        match serde_json::from_value(self.params) {
            Ok(params) => Ok((self.id, params)),
            Err(error) => Err(ExtractError::JsonError { method: self.method, error }),
        }
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        self.method == "shutdown"
    }
}

impl Notification {
    pub fn new(method: String, params: impl Serialize) -> Notification {
        Notification { method, params: serde_json::to_value(params).unwrap() }
    }
    pub fn extract<P: DeserializeOwned>(
        self,
        method: &str,
    ) -> Result<P, ExtractError<Notification>> {
        if self.method != method {
            return Err(ExtractError::MethodMismatch(self));
        }
        match serde_json::from_value(self.params) {
            Ok(params) => Ok(params),
            Err(error) => Err(ExtractError::JsonError { method: self.method, error }),
        }
    }
    pub(crate) fn is_exit(&self) -> bool {
        self.method == "exit"
    }
}

fn read_msg_text(inp: &mut dyn BufRead) -> io::Result<Option<String>> {
    fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, error)
    }
    macro_rules! invalid_data {
        ($($tt:tt)*) => (invalid_data(format!($($tt)*)))
    }

    let mut size = None;
    let mut buf = String::new();
    loop {
        buf.clear();
        if inp.read_line(&mut buf)? == 0 {
            return Ok(None);
        }
        if !buf.ends_with("\r\n") {
            return Err(invalid_data!("malformed header: {:?}", buf));
        }
        let buf = &buf[..buf.len() - 2];
        if buf.is_empty() {
            break;
        }
        let mut parts = buf.splitn(2, ": ");
        let header_name = parts.next().unwrap();
        let header_value =
            parts.next().ok_or_else(|| invalid_data!("malformed header: {:?}", buf))?;
        if header_name == "Content-Length" {
            size = Some(header_value.parse::<usize>().map_err(invalid_data)?);
        }
    }
    let size: usize = size.ok_or_else(|| invalid_data!("no Content-Length"))?;
    let mut buf = buf.into_bytes();
    buf.resize(size, 0);
    inp.read_exact(&mut buf)?;
    let buf = String::from_utf8(buf).map_err(invalid_data)?;
    // log::debug!("< {}", buf);
    Ok(Some(buf))
}

fn write_msg_text(out: &mut dyn Write, msg: &str) -> io::Result<()> {
    // log::debug!("> {}", msg);
    write!(out, "Content-Length: {}\r\n\r\n", msg.len())?;
    out.write_all(msg.as_bytes())?;
    out.flush()?;
    Ok(())
}
