//! One always-draining reader thread and one bounded writer thread per stream.
//!
//! stdin and stdout are separate pipes, so simultaneous writes are not inherently
//! deadlocking — but they are if either side does a blocking write-then-read while both
//! buffers fill. Nothing here ever reads and writes on the same thread.

use crate::proto::frame::{read_frame, write_frame, FrameError, MAX_FRAME_BYTES};
use crate::proto::txguard;
use crate::proto::wire::{Inbound, Outbound};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

/// Frames the writer may hold before a `send` blocks and a `try_send` reports `Full`.
pub const WRITER_CAPACITY: usize = 64;

static STDOUT_CLAIMED: AtomicBool = AtomicBool::new(false);

/// Hands out the process's stdout exactly once. Every later caller gets `None`, which is what
/// makes "stdout carries protocol frames and nothing else" checkable at runtime.
pub fn claim_stdout() -> Option<std::io::Stdout> {
    if STDOUT_CLAIMED.swap(true, Ordering::SeqCst) {
        None
    } else {
        Some(std::io::stdout())
    }
}

#[derive(Debug)]
pub enum SendError {
    /// A database transaction is open on this thread. Awaiting a pipe write here is the
    /// deadlock the process contract forbids.
    InTransaction,
    /// The bounded writer queue is full. Only `try_send` returns this.
    Full,
    /// The writer thread is gone.
    Closed,
    TooLarge {
        len: usize,
    },
    Encode(serde_json::Error),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InTransaction => write!(f, "pipe write attempted inside a transaction"),
            Self::Full => write!(f, "writer queue full"),
            Self::Closed => write!(f, "writer gone"),
            Self::TooLarge { len } => write!(f, "frame of {len} bytes exceeds the cap"),
            Self::Encode(e) => write!(f, "encode: {e}"),
        }
    }
}

impl std::error::Error for SendError {}

/// The only way to put bytes on stdout.
#[derive(Debug, Clone)]
pub struct FrameSink {
    tx: SyncSender<Vec<u8>>,
}

impl FrameSink {
    fn encode(frame: &Outbound) -> Result<Vec<u8>, SendError> {
        let bytes = serde_json::to_vec(frame).map_err(SendError::Encode)?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(SendError::TooLarge { len: bytes.len() });
        }
        Ok(bytes)
    }

    /// Blocks when the writer queue is full. Refuses outright inside a transaction.
    pub fn send(&self, frame: &Outbound) -> Result<(), SendError> {
        if txguard::in_transaction() {
            return Err(SendError::InTransaction);
        }
        let bytes = Self::encode(frame)?;
        self.tx.send(bytes).map_err(|_| SendError::Closed)
    }

    /// Never blocks. Backpressure decisions are made from its `Full`.
    pub fn try_send(&self, frame: &Outbound) -> Result<(), SendError> {
        if txguard::in_transaction() {
            return Err(SendError::InTransaction);
        }
        let bytes = Self::encode(frame)?;
        self.tx.try_send(bytes).map_err(|e| match e {
            TrySendError::Full(_) => SendError::Full,
            TrySendError::Disconnected(_) => SendError::Closed,
        })
    }
}

/// Both halves of the pipe, each on its own thread.
#[derive(Debug)]
pub struct Transport {
    pub inbound: Receiver<Inbound>,
    pub sink: FrameSink,
    writer: JoinHandle<Result<(), FrameError>>,
}

impl Transport {
    /// `capacity` bounds the writer queue; the inbound channel is unbounded so the reader
    /// never blocks and the pipe never fills from our side.
    pub fn start<R, W>(mut input: R, mut output: W, capacity: usize) -> std::io::Result<Self>
    where
        R: Read + Send + 'static,
        W: Write + Send + 'static,
    {
        let (in_tx, inbound) = channel::<Inbound>();
        let (out_tx, out_rx) = sync_channel::<Vec<u8>>(capacity);

        // The handle is dropped, detaching the thread: it parks in a blocking read on a pipe
        // that cannot be interrupted, and it dies with the process.
        drop(
            std::thread::Builder::new()
                .name("proto-reader".to_owned())
                .spawn(move || {
                    let mut buf = Vec::new();
                    loop {
                        read_frame(&mut input, &mut buf)?;
                        let Ok(msg) = serde_json::from_slice::<Inbound>(&buf) else {
                            return Err(FrameError::NotUtf8);
                        };
                        if in_tx.send(msg).is_err() {
                            return Ok::<(), FrameError>(());
                        }
                    }
                })?,
        );

        let writer = std::thread::Builder::new()
            .name("proto-writer".to_owned())
            .spawn(move || -> Result<(), FrameError> {
                while let Ok(bytes) = out_rx.recv() {
                    write_frame(&mut output, &bytes)?;
                }
                Ok(())
            })?;

        Ok(Self {
            inbound,
            sink: FrameSink { tx: out_tx },
            writer,
        })
    }

    /// Waits for the writer to drain. Every `FrameSink` clone must already be dropped, or
    /// this never returns. The reader is deliberately not joined: it is parked in a blocking
    /// read that only ends at the peer's EOF, which an orderly `app.shutdown` never produces.
    pub fn join(self) {
        drop(self.sink);
        let _ = self.writer.join();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{claim_stdout, SendError, Transport};
    use crate::proto::txguard::TxGuard;
    use crate::proto::wire::{Epoch, Inbound, Outbound, RequestId};
    use serde_json::json;
    use std::io::Write;

    struct Collect(std::sync::mpsc::Sender<Vec<u8>>);

    impl Write for Collect {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            let _ = self.0.send(b.to_vec());
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_frame_written_to_the_sink_lands_on_the_output_stream() {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let t = Transport::start(std::io::empty(), Collect(tx), 4).expect("transport");
        t.sink
            .send(&Outbound::Response {
                epoch: Epoch(1),
                id: RequestId(7),
                ok: json!({}),
            })
            .expect("send");
        let header = rx.recv().expect("header");
        assert_eq!(header.len(), 4, "length prefix first");
        let body = rx.recv().expect("body");
        assert!(std::str::from_utf8(&body)
            .expect("utf8")
            .contains("\"id\":7"));
        t.join();
    }

    #[test]
    fn a_send_inside_a_transaction_is_refused_rather_than_awaited() {
        let t = Transport::start(std::io::empty(), std::io::sink(), 1).expect("transport");
        let _open = TxGuard::enter();
        let e = t
            .sink
            .send(&Outbound::Response {
                epoch: Epoch(1),
                id: RequestId(1),
                ok: json!({}),
            })
            .expect_err("a pipe write under a database lock must be refused");
        assert!(matches!(e, SendError::InTransaction));
    }

    #[test]
    fn the_reader_drains_the_pipe_without_the_writer_being_read() {
        let mut wire = Vec::new();
        let body = serde_json::to_vec(&Inbound::Resync {
            topic: crate::protocol::Topic::Scan,
        })
        .expect("encode");
        crate::proto::frame::write_frame(&mut wire, &body).expect("frame");
        let t =
            Transport::start(std::io::Cursor::new(wire), std::io::sink(), 1).expect("transport");
        let got = t.inbound.recv().expect("one inbound frame");
        assert!(matches!(got, Inbound::Resync { .. }));
        t.join();
    }

    #[test]
    fn stdout_is_handed_out_once() {
        assert!(claim_stdout().is_some());
        assert!(
            claim_stdout().is_none(),
            "a second holder of stdout is a protocol breach"
        );
    }
}
