//! §20.10 audit 3 — the redaction audit, **and it is the gate: §20.5's rule without it is an
//! intention.**
//!
//! A connect is driven with a sentinel token and the sentinel is searched for in every place a
//! secret could surface. The three capture points are asserted in **one** test so a partial pass
//! is impossible.
//!
//! Plus AC-P2-20-12's structural half: **the shipped binary contains no OAuth client secret,
//! because none exists to contain.** That is the property that makes Device Flow the only OAuth
//! variant safe to ship in an open-source binary.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::{Arc, Mutex};

use codotheca_core::accounts::keychain::SecretToken;
use codotheca_core::http::{HttpResponse, HttpTransport};
use codotheca_core::testing::{FakeTokenStore, FakeTransport};

/// Obviously fake, and nothing like a real credential. It has to be greppable in a failure
/// message without a reader wondering whether a real token just reached their terminal.
const SENTINEL: &str = "SENTINEL-NOT-A-REAL-TOKEN-0000000000";

fn core_src() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let entry = entry.expect("a readable entry");
            let kind = entry.file_type().expect("an entry kind");
            let path = entry.path();
            if kind.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        let name = path
                            .strip_prefix(root)
                            .unwrap_or(&path)
                            .display()
                            .to_string()
                            .replace('\\', "/");
                        out.push((name, text));
                    }
                    // Skipped BEFORE it is counted, so "scanned nothing" keeps meaning what it
                    // says while another gate's probe file races this walk.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => panic!("{}: {e}", path.display()),
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&core_src(), &core_src(), &mut out);
    out
}

/// **AC-P2-20-4.** The sentinel reaches no rendered surface.
///
/// Three capture points, one test:
///
/// 1. **The command's own answer.** A connect returns an `Account`, and an `Account` carries no
///    token field at any depth — this searches the *serialised* value rather than a field list,
///    so a field added later fails here rather than slipping past.
/// 2. **Every error frame** the flow produced, serialised the same way. §20.5: a pasted token is
///    never quoted in an error message.
/// 3. **`diag.bundle`'s serialised bytes**, not selected fields, for the same reason.
///
/// Plus the stderr surface, which in-process means **`Debug`**: stderr is the rolling log's
/// channel and the only way a token reaches it is a `Debug` that prints one. The request that
/// carried the token is the value most likely to be printed while debugging, so its `Debug` is
/// asserted here rather than assumed.
#[test]
fn ac_p2_20_4_the_sentinel_appears_nowhere() {
    let dir = tempfile::tempdir().expect("tmp");
    let index = Arc::new(Mutex::new(
        codotheca_core::index::Index::open_at(dir.path(), 1_000).expect("index opens"),
    ));
    let transport = Arc::new(FakeTransport::new());
    transport.push(HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([("X-OAuth-Scopes", "read:user")]),
        body: br#"{"login":"octo","name":null}"#.to_vec(),
    });
    let tokens = FakeTokenStore::available();

    // 1. The answer.
    let http: Arc<dyn HttpTransport> = transport.clone();
    let account = codotheca_core::accounts::commands::connect_pat(
        &index,
        &http,
        &tokens,
        "forge.example.invalid",
        &SecretToken::new(SENTINEL.to_owned()),
        2_000,
    )
    .expect("the fixture authenticates");
    let answer = serde_json::to_string(&account).expect("an Account serialises");
    assert!(
        !answer.contains(SENTINEL),
        "the token reached the wire: {answer}"
    );

    // 2. Every error frame. Drive the same path to a failure and serialise what comes back.
    let failing = Arc::new(FakeTransport::new());
    failing.push(HttpResponse {
        status: 401,
        headers: Vec::new(),
        body: br#"{"message":"Bad credentials"}"#.to_vec(),
    });
    let failing_http: Arc<dyn HttpTransport> = failing;
    let failure = codotheca_core::accounts::commands::connect_pat(
        &index,
        &failing_http,
        &tokens,
        "forge.example.invalid",
        &SecretToken::new(SENTINEL.to_owned()),
        2_000,
    )
    .expect_err("a 401 fails");
    let frame = format!("{:?} {}", failure.code, failure.message);
    assert!(
        !frame.contains(SENTINEL),
        "the token reached an error frame: {frame}"
    );

    // 3. The whole diagnostics bundle, as bytes rather than as selected fields: a field added
    //    later must fail this test rather than slip past a field list.
    let guard = index
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let bundle =
        codotheca_core::surfaces::diag::build(guard.conn(), false, 900).expect("the bundle builds");
    drop(guard);
    let bytes = serde_json::to_string(&bundle).expect("the bundle serialises");
    assert!(!bytes.contains(SENTINEL), "the token reached diag.bundle");
    // The bundle must actually have content, or this assertion passes over nothing.
    assert!(bytes.len() > 2, "the bundle serialised to nothing");

    // The stderr surface: `Debug` on the request that carried the token.
    let sent = transport.requests();
    assert_eq!(sent.len(), 1, "the fixture issued exactly one request");
    let debug = format!("{:?}", sent[0]);
    assert!(
        !debug.contains(SENTINEL),
        "an HttpRequest prints its Authorization header, so any Debug of one leaks a token: \
         {debug}"
    );
    // And the token type itself.
    assert!(!format!("{:?}", SecretToken::new(SENTINEL.to_owned())).contains(SENTINEL));
}

/// **AC-P2-20-12.** The build carries no OAuth client secret, because none exists.
///
/// Two assertions, each printing what it scanned and failing at zero: no source file under
/// `core/src/` declares a client-secret-shaped constant, and the built binary's bytes contain no
/// such value.
#[test]
fn ac_p2_20_12_no_client_secret() {
    let sources = rust_sources();
    eprintln!(
        "accounts_redaction: scanned {} source file(s) for a client secret",
        sources.len()
    );
    assert!(
        !sources.is_empty(),
        "the walk read no file, so it proved nothing"
    );

    let mut offenders: Vec<String> = Vec::new();
    for (name, text) in &sources {
        for line in text.lines() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            let lower = code.to_ascii_lowercase();
            // A declaration, not a mention: `const X: &str = "…"` shaped, naming a secret.
            let declares = lower.contains("const ") || lower.contains("static ");
            let names_a_secret = lower.contains("client_secret")
                || lower.contains("clientsecret")
                || lower.contains("oauth_secret");
            if declares && names_a_secret {
                offenders.push(format!("{name}: {code}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a client secret is declared in the core: {offenders:?}"
    );

    // The client id is compiled in and is deliberately NOT a secret; assert it is declared, so
    // this test cannot pass by there being no OAuth code at all.
    assert!(
        sources
            .iter()
            .any(|(_, text)| text.contains("GITHUB_CLIENT_ID")),
        "no client id is declared anywhere, so the scan above proved nothing"
    );

    // The binary half. `CARGO_BIN_EXE_*` is set by cargo for an integration test, so this reads
    // the artefact that would ship rather than a rebuild of it.
    let binary = std::path::Path::new(env!("CARGO_BIN_EXE_codotheca-core"));
    let bytes = std::fs::read(binary).expect("the built core is readable");
    eprintln!(
        "accounts_redaction: scanned {} bytes of {} for a client secret",
        bytes.len(),
        binary.file_name().unwrap_or_default().to_string_lossy()
    );
    assert!(!bytes.is_empty(), "the binary was empty");
    // The **assignment** forms, not the bare token. A bare `client_secret` is not decidable
    // here: `rustls` emits `client_secrets` and `_client_secret` as TLS key-schedule symbol
    // names, and a scan for the word reports a dependency's debug symbols rather than anything
    // of ours. What only *we* could ship is a secret being sent — this core form-encodes
    // `client_id=…`, so a secret would appear as `client_secret=` or as a JSON field.
    for needle in [
        "client_secret=",
        "\"client_secret\"",
        "CLIENT_SECRET=",
        "oauth_secret=",
    ] {
        assert!(
            !bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes()),
            "the shipped binary sends {needle:?}"
        );
    }
}

/// Every type name reachable from `root`, following struct fields transitively.
///
/// `struct` is the only kind with fields — `enum` declares string variants and `id` a scalar
/// repr — so following struct fields walks the whole graph.
fn reachable_types(
    types: &serde_json::Map<String, serde_json::Value>,
    root: &str,
) -> std::collections::BTreeSet<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = vec![root.to_owned()];
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(decl) = types.get(&name) else {
            continue;
        };
        if decl["kind"] != "struct" {
            continue;
        }
        for expr in decl["fields"]
            .as_object()
            .expect("fields is an object")
            .values()
        {
            queue.push(base_type(expr));
        }
    }
    seen
}

fn base_type(expr: &serde_json::Value) -> String {
    expr.as_str()
        .expect("a type expression")
        .trim_start_matches('[')
        .trim_end_matches('?')
        .trim_end_matches(']')
        .to_owned()
}

fn schema_types() -> serde_json::Map<String, serde_json::Value> {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../protocol/schema/protocol.json"))
            .expect("protocol.json parses");
    schema["types"]
        .as_object()
        .expect("types is an object")
        .clone()
}

/// `Account`'s own fields are all primitives, ids and enums, so the credential test below cannot
/// prove the walk recurses: a `reachable_types` that queued nothing would walk `Account` and read
/// green. This proves the recursion on a type that actually nests.
#[test]
fn the_type_walk_follows_struct_fields_to_the_bottom() {
    let types = schema_types();
    let walked = reachable_types(&types, "Problems");
    for name in ["Problems", "ProblemGroup", "ProblemItem"] {
        assert!(walked.contains(name), "the walk stopped before {name}");
    }
}

/// AC-P2-20-4's second half, structural: **no `Account` value carries a token field at any
/// nesting depth.** Read from the schema and walked transitively, so a nested struct added later
/// fails here. `token_ref` deliberately does not appear — `Account` does not carry it.
#[test]
fn an_account_carries_no_credential_field_at_any_depth() {
    let types = schema_types();
    let mut fields_checked = 0_usize;
    for name in reachable_types(&types, "Account") {
        let Some(decl) = types.get(&name) else {
            continue;
        };
        if decl["kind"] != "struct" {
            continue;
        }
        for field in decl["fields"]
            .as_object()
            .expect("fields is an object")
            .keys()
        {
            fields_checked += 1;
            let lower = field.to_ascii_lowercase();
            assert!(
                !(lower.contains("token")
                    || lower.contains("secret")
                    || lower.contains("credential")),
                "{name}.{field} puts a credential on the wire"
            );
        }
    }
    eprintln!("accounts_redaction: walked {fields_checked} field(s) from Account");
    // Counting names would count `"String"`; counting fields counts what was actually tested.
    let declared = types["Account"]["fields"]
        .as_object()
        .expect("Account has fields")
        .len();
    assert!(
        fields_checked >= declared,
        "the walk tested {fields_checked} field(s), fewer than Account declares"
    );
}
