//! Integration test against the real built binary. This is the only place that proves
//! stdout carries frames and nothing else: it parses every byte the process emits.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use codotheca_core::proto::frame::{read_frame, write_frame};
use codotheca_core::proto::wire::{Epoch, Inbound, Outbound, RequestId};
use std::io::Write;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("codotheca-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn spawn(dir: &std::path::Path) -> std::process::Child {
    std::process::Command::new(env!("CARGO_BIN_EXE_codotheca-core"))
        .arg(format!("--data-dir={}", dir.display()))
        .arg("--epoch=7")
        .arg(format!("--parent-pid={}", std::process::id()))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn core")
}

fn send(child: &mut std::process::Child, frame: &Inbound) {
    let body = serde_json::to_vec(frame).expect("encode");
    let stdin = child.stdin.as_mut().expect("stdin");
    write_frame(stdin, &body).expect("write");
    stdin.flush().expect("flush");
}

#[test]
fn the_core_greets_acks_and_shuts_down_over_frames_only() {
    let dir = temp_dir("handshake");
    let mut child = spawn(&dir);

    let mut stdout = child.stdout.take().expect("stdout");
    let mut buf = Vec::new();
    read_frame(&mut stdout, &mut buf).expect("hello frame");
    let hello: Outbound = serde_json::from_slice(&buf).expect("hello decodes");
    match hello {
        Outbound::Hello {
            protocol_version,
            epoch,
            ..
        } => {
            assert_eq!(protocol_version, codotheca_core::protocol::PROTOCOL_VERSION);
            assert_eq!(
                epoch,
                Epoch(7),
                "the shell's epoch is echoed, not reinvented"
            );
        }
        other => panic!("the first frame must be hello, got {other:?}"),
    }

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

    send(
        &mut child,
        &Inbound::Request {
            id: RequestId(1),
            command: "app.shutdown".to_owned(),
            args: serde_json::json!({}),
        },
    );

    // Every remaining byte on stdout must still be a frame — no stray print, no panic text.
    loop {
        match read_frame(&mut stdout, &mut buf) {
            Ok(()) => {
                serde_json::from_slice::<Outbound>(&buf).expect("stdout carried a non-frame");
            }
            Err(codotheca_core::proto::frame::FrameError::Eof) => break,
            Err(e) => panic!("stdout was not frames all the way down: {e}"),
        }
    }

    let status = child.wait().expect("wait");
    assert_eq!(status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unknown_command_is_refused_without_killing_the_connection() {
    let dir = temp_dir("unknown");
    let mut child = spawn(&dir);
    let mut stdout = child.stdout.take().expect("stdout");
    let mut buf = Vec::new();
    read_frame(&mut stdout, &mut buf).expect("hello");

    send(
        &mut child,
        &Inbound::Request {
            id: RequestId(4),
            command: "nope.notacommand".to_owned(),
            args: serde_json::json!({}),
        },
    );
    read_frame(&mut stdout, &mut buf).expect("error frame");
    let frame: Outbound = serde_json::from_slice(&buf).expect("decode");
    match frame {
        Outbound::Error {
            id, code, outcome, ..
        } => {
            assert_eq!(id, RequestId(4));
            assert_eq!(code, codotheca_core::protocol::ErrorCode::Protocol);
            // R31: no `Failed` variant exists — an unknown command definitely did nothing,
            // and that is `None`.
            assert_eq!(outcome, None);
        }
        other => panic!("expected an error frame, got {other:?}"),
    }

    // Still alive: a second command still gets an answer.
    send(
        &mut child,
        &Inbound::Request {
            id: RequestId(5),
            command: "app.shutdown".to_owned(),
            args: serde_json::json!({}),
        },
    );
    let status = child.wait().expect("wait");
    assert_eq!(status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_core_on_one_data_dir_exits_with_the_lock_code() {
    let dir = temp_dir("lock");
    let mut first = spawn(&dir);
    let mut stdout = first.stdout.take().expect("stdout");
    let mut buf = Vec::new();
    read_frame(&mut stdout, &mut buf).expect("first core is up");

    let second = spawn(&dir).wait_with_output().expect("second core");
    assert_eq!(
        second.status.code(),
        Some(i32::from(codotheca_core::lifecycle::EXIT_LOCK_HELD))
    );
    assert!(
        second.stdout.is_empty(),
        "a refusal writes nothing to stdout"
    );
    assert!(
        !second.stderr.is_empty(),
        "the reason goes to stderr, where the shell reads it"
    );

    let _ = first.kill();
    let _ = first.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_argv_exits_with_the_args_code_and_says_why_on_stderr() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_codotheca-core"))
        .arg("--epoch=1")
        .output()
        .expect("run");
    assert_eq!(
        out.status.code(),
        Some(i32::from(codotheca_core::lifecycle::EXIT_BAD_ARGS))
    );
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--data-dir"));
}
