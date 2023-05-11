
use std::{
    io::{self, BufReader, stdin, stdout},
    thread,
    fmt,
    net::{TcpListener, TcpStream, ToSocketAddrs},
};
use crossbeam_channel::{bounded, Receiver, Sender};

use crate::msg::{Message, Request, Response};


pub struct Connection {
    pub sender: Sender<Message>,
    pub receiver: Receiver<Message>,
}

#[derive(Debug, Clone)]
pub struct ProtocolError(pub(crate) String);

impl std::error::Error for ProtocolError {}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

pub struct IoThreads {
    reader: thread::JoinHandle<io::Result<()>>,
    writer: thread::JoinHandle<io::Result<()>>,
}

pub(crate) fn make_io_threads(
    reader: thread::JoinHandle<io::Result<()>>,
    writer: thread::JoinHandle<io::Result<()>>,
) -> IoThreads {
    IoThreads { reader, writer }
}

impl IoThreads {
    pub fn join(self) -> io::Result<()> {
        match self.reader.join() {
            Ok(r) => r?,
            Err(err) => {
                println!("reader panicked!");
                std::panic::panic_any(err)
            }
        }
        match self.writer.join() {
            Ok(r) => r,
            Err(err) => {
                println!("writer panicked!");
                std::panic::panic_any(err);
            }
        }
    }
}

pub(crate) fn stdio_transport() -> (Sender<Message>, Receiver<Message>, IoThreads) {
    let (writer_sender, writer_receiver) = bounded::<Message>(0);
    let writer = thread::spawn(move || {
        let stdout = stdout();
        let mut stdout = stdout.lock();
        writer_receiver.into_iter().try_for_each(|it| it.write(&mut stdout))?;
        Ok(())
    });
    let (reader_sender, reader_receiver) = bounded::<Message>(0);
    let reader = thread::spawn(move || {
        let stdin = stdin();
        let mut stdin = stdin.lock();
        while let Some(msg) = Message::read(&mut stdin)? {
            let is_exit = match &msg {
                Message::Notification(n) => n.is_exit(),
                _ => false,
            };

            reader_sender.send(msg).unwrap();

            if is_exit {
                break;
            }
        }
        Ok(())
    });
    let threads = IoThreads { reader, writer };
    (writer_sender, reader_receiver, threads)
}

pub(crate) fn socket_transport(
    stream: TcpStream,
) -> (Sender<Message>, Receiver<Message>, IoThreads) {
    let (reader_receiver, reader) = make_reader(stream.try_clone().unwrap());
    let (writer_sender, writer) = make_write(stream);
    let io_threads = make_io_threads(reader, writer);
    (writer_sender, reader_receiver, io_threads)
}

fn make_reader(stream: TcpStream) -> (Receiver<Message>, thread::JoinHandle<io::Result<()>>) {
    let (reader_sender, reader_receiver) = bounded::<Message>(0);
    let reader = thread::spawn(move || {
        let mut buf_read = BufReader::new(stream);
        while let Some(msg) = Message::read(&mut buf_read).unwrap() {
            let is_exit = matches!(&msg, Message::Notification(n) if n.is_exit());
            reader_sender.send(msg).unwrap();
            if is_exit {
                break;
            }
        }
        Ok(())
    });
    (reader_receiver, reader)
}

fn make_write(mut stream: TcpStream) -> (Sender<Message>, thread::JoinHandle<io::Result<()>>) {
    let (writer_sender, writer_receiver) = bounded::<Message>(0);
    let writer = thread::spawn(move || {
        writer_receiver.into_iter().try_for_each(|it| it.write(&mut stream)).unwrap();
        Ok(())
    });
    (writer_sender, writer)
}


impl Connection {
    /// Create connection over standard in/standard out.
    ///
    /// Use this to create a real language server.
    pub fn stdio() -> (Connection, IoThreads) {
        let (sender, receiver, io_threads) = stdio_transport();
        (Connection { sender, receiver }, io_threads)
    }

    /// Open a connection over tcp.
    /// This call blocks until a connection is established.
    ///
    /// Use this to create a real language server.
    pub fn connect<A: ToSocketAddrs>(addr: A) -> io::Result<(Connection, IoThreads)> {
        let stream = TcpStream::connect(addr)?;
        let (sender, receiver, io_threads) = socket_transport(stream);
        Ok((Connection { sender, receiver }, io_threads))
    }

    /// Listen for a connection over tcp.
    /// This call blocks until a connection is established.
    ///
    /// Use this to create a real language server.
    pub fn listen<A: ToSocketAddrs>(addr: A) -> io::Result<(Connection, IoThreads)> {
        let listener = TcpListener::bind(addr)?;
        let (stream, _) = listener.accept()?;
        let (sender, receiver, io_threads) = socket_transport(stream);
        Ok((Connection { sender, receiver }, io_threads))
    }

    /// Creates a pair of connected connections.
    ///
    /// Use this for testing.
    pub fn memory() -> (Connection, Connection) {
        let (s1, r1) = crossbeam_channel::unbounded();
        let (s2, r2) = crossbeam_channel::unbounded();
        (Connection { sender: s1, receiver: r2 }, Connection { sender: s2, receiver: r1 })
    }

    /// If `req` is `Shutdown`, respond to it and return `true`, otherwise return `false`
    pub fn handle_shutdown(&self, req: &Request) -> Result<bool, ProtocolError> {
        if !req.is_shutdown() {
            return Ok(false);
        }
        let resp = Response::new_ok(req.id.clone(), ());
        let _ = self.sender.send(resp.into());
        match &self.receiver.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(Message::Notification(n)) if n.is_exit() => (),
            Ok(msg) => {
                return Err(ProtocolError(format!("unexpected message during shutdown: {msg:?}")))
            }
            Err(e) => return Err(ProtocolError(format!("unexpected error during shutdown: {e}"))),
        }
        Ok(true)
    }
}

