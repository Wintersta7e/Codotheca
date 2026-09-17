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
//! Most schema commands reach a module. The two remaining `install.*` commands route to
//! `NoOwner`, and `UNOWNED_COMMANDS` names the later task responsible for each. A bare `PROTOCOL`
//! refusal reads to the shell as "no such command", which is how nineteen commands stayed
//! invisible for the length of this project. This file's tests pin the constant against
//! `protocol.json` and against `route`'s own arms, so the two cannot be edited apart — including
//! when the list eventually becomes empty.
//!
//! **A command leaves the list in the same change that gives it a handler.** The bridge's own
//! `KNOWN_COMMANDS` is checked against this list, so a name cannot be offered to the renderer
//! while it is still unowned.
//!
//! Two things the dispatcher still cannot do, recorded so they are not rediscovered:
//!
//! - **No job runner is constructed, so no job is scheduled.** `rusqlite::Connection` is `Send`
//!   but not `Sync`; the index is shared as `Arc<Mutex<Index>>` and the tick runs on the loop
//!   thread. `scan.start` reaches a handler and enqueues nothing across a thread either — the
//!   `ScanLauncher` seam is `Send + Sync` precisely so no connection crosses it.
//! - **The production `DistroProbe` is now wired.** `wsl::distros::SystemDistroProbe` implements
//!   it over `SystemWslCli` and the composition root passes that. It reads the two quiet
//!   `wsl.exe` listings — neither of which names a distro, so neither can start one — and
//!   answers with an empty list where there is no `wsl.exe`.

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
    /// `crate::remote::dispatch_remote_command` — p2-25. It reads one stored key and the account
    /// hosts and reaches no network, so it takes the index guard like every other read.
    Remote,
    /// `crate::install::handle_preview` — p2-24 Task 10. It takes its own index guard before its
    /// read transaction, so it is dispatched before the common guarded arm.
    Install,
    /// `crate::readme::dispatch_readme_command` — p2-25b. A file read under a location root and
    /// a consent column; no network, so it takes the index guard.
    Readme,
    /// `projects.readmeAssets`, answered **without** the index guard (R75). Same carve-out as
    /// [`Route::Scan`] and [`Route::AccountsNet`], for the same reason: it fetches up to 24
    /// remote assets of 5 s each, and the one SQLite mutex may not be held across them.
    ReadmeNet,
    /// `crate::sync::commands::dispatch_sync_command` — p2-21. A read of two tables plus the
    /// runner's own process state; it reaches no network, so it takes the index guard like every
    /// other read (R94's first side).
    Sync,
    /// `crate::accounts::dispatch_accounts_command` — p2-20, under the index guard.
    Accounts,
    /// The `accounts.*` commands that reach the network, answered **without** the index guard
    /// (R75). Same carve-out as [`Route::Scan`], for the same reason.
    AccountsNet,
    /// In §2.4 and in the schema, with no module in any plan. The payload names the plan that
    /// owes it, so the diagnostic says who, not just that.
    NoOwner(&'static str),
}

/// The commands with no handler, and the plan each belongs to.
///
/// **A command leaves this list in the same change that gives it a handler.** The test in this
/// file compares it against the arms of `route`, so the two cannot drift.
///
/// It is the seam that makes an unhandled command *named and refused* instead of mis-routed: a
/// `PROTOCOL` refusal reads to the shell as "no such command", and nineteen commands hid behind
/// exactly that for the length of this project. A schema command with no module goes here, with
/// its plan, in the same change that adds its `NoOwner` arm.
///
/// The eight `accounts.*` rows landed with the schema and left as their handlers did. §24.9's
/// three `install.*` rows arrived the same way and have all now left, the last of them with the
/// process-group kill §24.3c needs. Empty is a state to assert, not a state
/// to stop asserting: the test below reads the router rather than this list.
/// **Empty, and that is a state to assert rather than a state to stop asserting.** The test
/// below reads the router rather than this list, so an empty constant still proves that no
/// command routes to `NoOwner` — two `all()` calls over an empty set assert nothing.
pub const UNOWNED_COMMANDS: [(&str, &str); 2] = [
    ("locations.uninstallPreflight", "p2-24b Task 10"),
    ("locations.uninstall", "p2-24b Task 11"),
];

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

        // §25.2's opener, answered from the stored `remote_key` and the account hosts.
        CommandName::RemoteWebUrl => Route::Remote,

        // §21.13's one command. A read; the runner does the network, never this arm.
        CommandName::SyncStatus => Route::Sync,

        // §25.5's document read and its consent write.
        CommandName::ProjectsReadme | CommandName::ProjectsSetReadmeRemote => Route::Readme,

        // §25.5's asset read, which reaches arbitrary hosts and therefore takes no guard.
        CommandName::ProjectsReadmeAssets => Route::ReadmeNet,

        // The two §20.8 reads, answered by `crate::accounts` **under the index guard**. These
        // two only read a row; every other `accounts.*` command is below.
        CommandName::AccountsList | CommandName::AccountsOrgs => Route::Accounts,

        // R75: everything that makes a call this process cannot bound is answered **without the
        // index lock**, like `Route::Scan`. Holding the one SQLite mutex across such a call stops
        // every other command for its whole duration.
        //
        // The unbounded call is not always the forge. `setOrgEnabled` preflights it for as long
        // as `ACCOUNT_LIMITS.total_secs`; `disconnect` makes a **keychain** round trip, which
        // `keyring` puts no timeout on at all — a locked credential store can prompt and a
        // secret-service call can wait on D-Bus. Both were answered under the guard.
        CommandName::AccountsConnect
        | CommandName::AccountsCancelConnect
        | CommandName::AccountsConnectPat
        | CommandName::AccountsUpgradeScope
        | CommandName::AccountsSetOrgEnabled
        | CommandName::AccountsDisconnect => Route::AccountsNet,

        // §24.9's three, all answered by Install.
        CommandName::InstallPreview | CommandName::InstallStart | CommandName::InstallCancel => {
            Route::Install
        }

        // [p2] §24.7's two, declared with the schema and answered by the uninstall runtime. Each
        // names the task that owes it, so the refusal says *who* rather than only *that*.
        CommandName::LocationsUninstallPreflight => Route::NoOwner("p2-24b Task 10"),
        CommandName::LocationsUninstall => Route::NoOwner("p2-24b Task 11"),
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
    fn every_schema_command_reaches_a_module_or_names_the_task_that_owes_it() {
        // R37 closed Gap A and the list was empty; §20.8 reopened it with the eight commands
        // whose handlers landed later in the same plan, and §24.9 reopens it with three. **An
        // empty list is not a licence to stop asserting**: `UNOWNED_COMMANDS.iter().all(…)` over
        // zero rows is true whatever the rule says, so what is checked here is read off the
        // router — no schema command falls through to a `NoOwner` that names nobody, and no row
        // names a different task from the one the router hands the dispatcher. The sibling test
        // above compares the two name sets; this one compares the payloads, which is where a
        // rename would otherwise pass unnoticed.
        let commands = schema_commands();
        // R67 names three assertions that go red on a phase-2 schema change; this is a fourth,
        // and it is raised by each plan's own delta read from the branch base — never to a
        // running total a lane cannot know after the merges ahead of it.
        assert_eq!(
            commands.len(),
            60,
            "the schema this plan routes, §2.4 plus R33 gap 1 plus §20.8's eight plus §25.8's \
             remote.webUrl plus §25.8's three projects.readme* commands plus §24.9's three \
             install.* commands plus §21.13's sync.status plus §24.7's two locations.uninstall* \
             commands"
        );
        let unnamed: Vec<&str> = commands
            .iter()
            .filter(|name| match route(command_name(name).expect("routable")) {
                Route::NoOwner(plan) => !UNOWNED_COMMANDS
                    .iter()
                    .any(|(c, p)| *c == name.as_str() && *p == plan && !p.is_empty()),
                _ => false,
            })
            .map(String::as_str)
            .collect();
        assert!(
            unnamed.is_empty(),
            "a schema command reaches no module and no row names its owner: {unnamed:?}"
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
