//! The generated bindings are only correct if they produce the bytes the shell reads. Field
//! renaming, byte tagging and command tagging are all serde attributes, and an attribute that is
//! wrong compiles perfectly.

// `expect_used` and `panic` are denied crate-wide because a panic in the core kills the process
// the shell supervises. That reasoning inverts in a test binary: a failed round-trip must abort
// the test loudly, and the alternative — propagating Result out of every case — hides the
// assertion behind error plumbing. These allows belong at the top of every file in core/tests/.
#![allow(clippy::expect_used, clippy::panic)]

use codotheca_core::protocol::{AppHelloAckArgs, Bytes, Command, LocationId, LocationRef};

#[test]
fn a_command_is_tagged_by_name_with_its_args() {
    let c = Command::AppHelloAck(AppHelloAckArgs {
        protocol_version: 2,
    });
    let json = serde_json::to_string(&c).expect("serialise");
    assert_eq!(
        json,
        r#"{"command":"app.hello_ack","args":{"protocolVersion":2}}"#
    );
    let back: Command = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(back, c);
}

#[test]
fn struct_fields_cross_as_camel_case() {
    let r = LocationRef {
        id: LocationId(7),
        path_display: "a/b".to_owned(),
    };
    assert_eq!(
        serde_json::to_string(&r).expect("serialise"),
        r#"{"id":7,"pathDisplay":"a/b"}"#
    );
}

#[test]
fn bytes_cross_tagged_and_round_trip_non_utf8() {
    // §1.3: Linux paths are arbitrary bytes and cannot round-trip through TEXT or JSON.
    // This sequence is not valid UTF-8.
    let raw = vec![0x2f, 0x74, 0xff, 0xfe, 0x2f, 0x78];
    let json = serde_json::to_string(&Bytes(raw.clone())).expect("serialise");
    assert_eq!(json, r#"{"b64":"L3T//i94"}"#);
    let back: Bytes = serde_json::from_str(&json).expect("deserialise");
    assert_eq!(back.0, raw);
}

#[test]
fn an_unknown_field_is_a_protocol_error_not_a_silent_drop() {
    let bad = r#"{"protocolVersion":2,"extra":1}"#;
    assert!(serde_json::from_str::<AppHelloAckArgs>(bad).is_err());
}

#[test]
fn the_protocol_version_is_the_schema_version() {
    assert_eq!(codotheca_core::protocol::PROTOCOL_VERSION, 2);
}

#[test]
fn a_privileged_command_is_the_only_shape_carrying_bytes_inbound() {
    // §2.4's trust rule as the core sees it: a bytes-bearing frame must name one of the three
    // privileged commands. The generated Command enum is what makes that checkable at all.
    let frame =
        r#"{"command":"roots.add","args":{"pathBytes":{"b64":"L3RtcA=="},"confirmLarge":false}}"#;
    let c: Command = serde_json::from_str(frame).expect("deserialise");
    match c {
        Command::RootsAdd(a) => {
            assert_eq!(a.path_bytes.0, b"/tmp");
            assert!(!a.confirm_large);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn an_unknown_value_crosses_as_null_and_never_as_absence() {
    // §1.10: never render unknown as zero. On the wire that means never omitting the key either —
    // an absent key and a zero are equally unreadable as "not computed" at the receiver.
    use codotheca_core::protocol::{ProjectConditionChanged, ProjectId};
    let e = ProjectConditionChanged {
        id: ProjectId(1),
        condition_signal: None,
    };
    assert_eq!(
        serde_json::to_string(&e).expect("serialise"),
        r#"{"id":1,"conditionSignal":null}"#
    );
}
