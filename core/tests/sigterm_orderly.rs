//! `AC-P4-50-7`'s core half: a SIGTERM leaves the core's shutdown to the shell. A logout or a
//! launcher signals the shell's whole process group, the core with it; the shell then stops the
//! core by closing its stdin, and the orderly close only runs if the core is still there to run it.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::io::{Read as _, Write as _};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use codotheca_core::proto::frame::{read_frame, write_frame};
use codotheca_core::proto::wire::{Inbound, Outbound, RequestId};

fn send(child: &mut Child, frame: &Inbound) {
    let body = serde_json::to_vec(frame).expect("encode");
    let stdin = child.stdin.as_mut().expect("stdin");
    write_frame(stdin, &body).expect("write");
    stdin.flush().expect("flush");
}

#[test]
fn ac_p4_50_7_sigterm_does_not_stop_the_core() {
    let dir = tempfile::tempdir().expect("data dir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_codotheca-core"))
        .arg(format!("--data-dir={}", dir.path().display()))
        .arg("--epoch=3")
        .arg(format!("--parent-pid={}", std::process::id()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn core");
    // Drained on its own thread, so a full stderr pipe can never stall the core under test.
    let mut stderr = child.stderr.take().expect("stderr");
    let log = std::thread::spawn(move || {
        let mut text = String::new();
        stderr.read_to_string(&mut text).expect("stderr");
        text
    });

    let mut stdout = child.stdout.take().expect("stdout");
    let mut buf = Vec::new();
    read_frame(&mut stdout, &mut buf).expect("hello frame");
    assert!(matches!(
        serde_json::from_slice(&buf).expect("hello decodes"),
        Outbound::Hello { .. }
    ));
    send(
        &mut child,
        &Inbound::Request {
            id: RequestId(0),
            command: "app.hello_ack".to_owned(),
            args: serde_json::json!({
                "protocolVersion": codotheca_core::protocol::PROTOCOL_VERSION
            }),
        },
    );
    read_frame(&mut stdout, &mut buf).expect("ack response");

    let kill = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .expect("run kill");
    assert!(kill.success(), "kill -TERM was refused");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        child.try_wait().expect("poll the core"),
        None,
        "the core died of the SIGTERM instead of waiting for the shell"
    );

    drop(child.stdin.take());
    // Every remaining byte on stdout is read, so the core's writer never blocks on a full pipe.
    while read_frame(&mut stdout, &mut buf).is_ok() {}
    let status = child.wait().expect("wait");
    let log = log.join().expect("stderr thread");
    assert_eq!(status.code(), Some(0), "stderr:\n{log}");
    assert!(
        log.contains("codotheca-core: exiting (StdinEof)"),
        "no orderly close on stdin EOF; stderr:\n{log}"
    );
}
