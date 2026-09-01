//! Which module answers which command.
//!
//! The `match` in `route` has **no wildcard arm**. Adding a command to
//! `protocol/schema/protocol.json` regenerates `CommandName` with a new variant and this file
//! stops compiling — `error[E0004]: non-exhaustive patterns`. That is deliberate: an
//! unhandled command must fail the build, not fall through to a `PROTOCOL` refusal a user
//! discovers at runtime.
//!
//! # The ledger, as it stands
//!
//! **All 42 schema commands reach a module; `UNOWNED_COMMANDS` is empty.** It stays empty rather
//! than being deleted: it is what *names* the next command that arrives without a handler, and a
//! bare `PROTOCOL` refusal reads to the shell as "no such command" — which is how nineteen
//! commands stayed invisible for the length of this project. This file's tests pin the constant
//! against `protocol.json` and against `route`'s own arms, so the two cannot be edited apart.
//!
//! Two things the dispatcher still cannot do, recorded so they are not rediscovered:
//!
//! - **No job runner is constructed, so no job is scheduled.** `rusqlite::Connection` is `Send`
//!   but not `Sync`; the index is shared as `Arc<Mutex<Index>>` and the tick runs on the loop
//!   thread. `scan.start` reaches a handler and enqueues nothing across a thread either — the
//!   `ScanLauncher` seam is `Send + Sync` precisely so no connection crosses it.
//! - **No production `DistroProbe`.** `SystemWslCli` exists and nothing implements the trait over
//!   it; the composition root passes `NoDistros`, which reports *no distros found* rather than
//!   guessing. Wiring it is the WSL worker's.

use crate::proto::dispatch::CommandFailure;
use crate::protocol::CommandName;
use serde::de::IntoDeserializer as _;
use serde::Deserialize as _;

/// The module that answers a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// `run_loop` answers it before the handler is consulted (§2.2's handshake pair).
    Loop,
    /// `crate::firstrun::dispatch` — plan 16.
    FirstRun,
    /// `crate::art::dispatch_art_command` — plan 10b.
    Art,
    /// `crate::commands::launch::dispatch_launch_command` — plan 11c.
    Launch,
    /// `crate::commands::targets::dispatch_targets_command` — plan 11c.
    Targets,
    /// `crate::surfaces::dispatch_surface_command` — plan 17.
    Surfaces,
    /// `crate::scan::dispatch_scan_command` — plan 07.
    Scan,
    /// `crate::identity::commands::dispatch_identity_command` — plan 08.
    Identity,
    /// `crate::projects::dispatch_projects_command` — plan 13.
    Projects,
    /// `crate::detail::dispatch_detail_command` — plan 14.
    Detail,
    /// `crate::view::dispatch_view_command` — plan 15.
    View,
    /// In §2.4 and in the schema, with no module in any plan. The payload names the plan that
    /// owes it, so the diagnostic says who, not just that.
    NoOwner(&'static str),
}

/// The commands with no handler, and the plan each belongs to.
///
/// **A command leaves this list in the same change that gives it a handler.** The test in this
/// file compares it against the arms of `route`, so the two cannot drift.
///
/// **Empty since R37**, and deliberately not deleted. It is the seam that makes an unhandled
/// command *named and refused* instead of mis-routed: a `PROTOCOL` refusal reads to the shell as
/// "no such command", and nineteen commands hid behind exactly that for the length of this
/// project. The next schema command with no module goes here, with its plan, in the same change
/// that adds its `NoOwner` arm.
pub const UNOWNED_COMMANDS: [(&str, &str); 0] = [];

/// The wire name of a command into the generated enum.
///
/// The wire spelling lives in the generated `#[serde(rename = …)]` and nowhere else, so this
/// goes through serde rather than a second table of forty-two strings — the same technique
/// `PublisherSink::emit` uses for `Topic`.
pub fn command_name(command: &str) -> Result<CommandName, CommandFailure> {
    let de: serde::de::value::StrDeserializer<'_, serde::de::value::Error> =
        command.into_deserializer();
    CommandName::deserialize(de)
        .map_err(|_| CommandFailure::protocol(format!("unknown command {command:?}")))
}

/// Total. No wildcard arm — see the module comment.
#[must_use]
pub fn route(command: CommandName) -> Route {
    match command {
        CommandName::AppHelloAck | CommandName::AppShutdown => Route::Loop,

        // The nine of `crate::firstrun::FIRST_RUN_COMMANDS`, and exactly those (R37, R33 gap 1).
        CommandName::RootsSuggest
        | CommandName::RootsList
        | CommandName::RootsAdd
        | CommandName::RootsRemove
        | CommandName::RootsSetEnabled
        | CommandName::RootsSetDescend
        | CommandName::StatsReveal
        | CommandName::IdentityList
        | CommandName::IdentityConfirm => Route::FirstRun,

        CommandName::ArtUrl | CommandName::ArtRerender => Route::Art,

        CommandName::ProjectsLaunch | CommandName::SessionStop | CommandName::SessionFocus => {
            Route::Launch
        }

        CommandName::TargetsList
        | CommandName::TargetsSetDefault
        | CommandName::TargetsUpsert
        | CommandName::TargetsVerify => Route::Targets,

        CommandName::ProblemsList
        | CommandName::SettingsGet
        | CommandName::SettingsSet
        | CommandName::LocationsSetTrusted
        | CommandName::ProjectsRequeue
        | CommandName::DiagBundle => Route::Surfaces,

        CommandName::ScanStart | CommandName::ScanCancel | CommandName::ScanStatus => Route::Scan,

        CommandName::ProjectsMerge | CommandName::ProjectsUnmergeHint => Route::Identity,

        CommandName::ProjectsList | CommandName::ProjectsPeek | CommandName::ProjectsSetFlags => {
            Route::Projects
        }

        CommandName::ProjectsGet
        | CommandName::ProjectsSetNote
        | CommandName::LocationsRelocate => Route::Detail,

        CommandName::ViewGet
        | CommandName::ViewSet
        | CommandName::CollectionsList
        | CommandName::CollectionsUpsert
        | CommandName::CollectionsRemove => Route::View,
        // No `NoOwner` arm today (R37). One returns the moment a schema command lands without a
        // module, and `UNOWNED_COMMANDS` gains its row in the same edit.
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The schema, read from disk, is the only list of commands that exists at test time.
    fn schema_commands() -> Vec<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../protocol/schema/protocol.json");
        let text = std::fs::read_to_string(&path).expect("protocol.json is readable");
        let doc: serde_json::Value = serde_json::from_str(&text).expect("protocol.json parses");
        doc["commands"]
            .as_array()
            .expect("commands is an array")
            .iter()
            .map(|c| {
                c["name"]
                    .as_str()
                    .expect("every command has a name")
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn every_schema_command_deserialises_into_the_generated_enum() {
        for name in schema_commands() {
            command_name(&name)
                .unwrap_or_else(|e| panic!("{name} is not a CommandName: {}", e.message));
        }
    }

    #[test]
    fn every_schema_command_has_a_route() {
        // `route` is total by construction; this asserts the three lists agree, which the
        // compiler cannot: schema -> generated enum -> route arm.
        for name in schema_commands() {
            let cmd = command_name(&name).expect("routable");
            let _ = route(cmd);
        }
    }

    #[test]
    fn unowned_commands_is_exactly_the_set_routed_to_no_owner() {
        let declared: BTreeSet<&str> = UNOWNED_COMMANDS.iter().map(|(c, _)| *c).collect();
        let mut routed: BTreeSet<String> = BTreeSet::new();
        for name in schema_commands() {
            let cmd = command_name(&name).expect("routable");
            if matches!(route(cmd), Route::NoOwner(_)) {
                routed.insert(name);
            }
        }
        let routed: BTreeSet<&str> = routed.iter().map(String::as_str).collect();
        assert_eq!(declared, routed, "Gap A's table and the router disagree");
    }

    #[test]
    fn nothing_is_unowned_today_and_the_refusal_is_still_wired() {
        // R37 closed Gap A, so both sides of the test above are empty. The constant and its
        // `NoOwner` arm stay: they are what *names* the next command that reaches the schema
        // with no module. Without them a half-landed command falls back to a PROTOCOL refusal
        // the shell reads as "no such command", which is how nineteen of them stayed invisible.
        assert!(
            UNOWNED_COMMANDS.is_empty(),
            "a command joined the list without an arm in route"
        );
        assert_eq!(
            schema_commands().len(),
            42,
            "the schema this plan routes, §2.4 plus R33 gap 1"
        );
    }

    #[test]
    fn the_two_handshake_commands_are_the_loops_and_nothing_else_is() {
        let mut loops: Vec<String> = Vec::new();
        for name in schema_commands() {
            if route(command_name(&name).expect("routable")) == Route::Loop {
                loops.push(name);
            }
        }
        loops.sort();
        assert_eq!(loops, ["app.hello_ack", "app.shutdown"]);
    }

    #[test]
    fn an_unknown_command_string_is_a_protocol_error_not_an_internal_one() {
        let e = command_name("nope.notacommand").expect_err("must refuse");
        assert_eq!(e.code, crate::protocol::ErrorCode::Protocol);
        assert_eq!(
            e.outcome, None,
            "a name that was never dispatched did not take effect"
        );
    }
}
