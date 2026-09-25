//! §47.9 B: every git config key that can change what a write intent does, classified.
//!
//! **Config decides what a verb writes, not argv alone** — a user's refspec, `fetch.prune` and
//! `pruneTags` made the audited fetch delete a checked-out branch while every argv assertion
//! passed (§47.1). So each key `git help --config` lists in the audited namespaces carries a
//! class, and `core/tests/git_config_tripwire.rs` fails on any key the gate's git lists that this
//! table does not — a newer git adding a key is caught by the gate that runs it. **`push.*` is not
//! audited: no intent pushes** (U1).
//!
//! Keys are spelled as `git help --config` prints them, placeholders included. The classes are
//! Lane 0's, over `{Clone, VerifyRead}`; §47.9 C's differential layer proves each *inert* and
//! *pinned* reading on disk. **No lookup function lands here**: its first production reader is
//! `TagArchived`'s runtime guard, which lands with that intent — a lookup with no caller would be
//! a producer with no production path.

use crate::gitw::intent::IntentKind;

/// The nineteen namespaces every key of which is audited (§47.9 B).
pub const AUDITED_NAMESPACES: [&str; 19] = [
    "fetch",
    "remote",
    "tag",
    "transfer",
    "submodule",
    "url",
    "protocol",
    "gc",
    "maintenance",
    "bundle",
    "gpg",
    "include",
    "includeIf",
    "clone",
    "init",
    "credential",
    "http",
    "user",
    "filter",
];

/// The six `core.*` keys audited on their own (§47.9 B).
pub const AUDITED_CORE_KEYS: [&str; 6] = [
    "core.hooksPath",
    "core.fsmonitor",
    "core.logAllRefUpdates",
    "core.sshCommand",
    "core.askPass",
    "core.gitProxy",
];

/// How one config key bears on the write intents (§47.9 B's four classes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyClass {
    /// Overridden on every child it could affect, by the pin named.
    Pinned {
        /// The argv or environment pin that overrides it.
        by: &'static str,
    },
    /// The named intents refuse to run while it is set (§47.7's runtime guard).
    Refused {
        /// The intents that refuse.
        intents: &'static [IntentKind],
        /// Why a pin cannot neutralise it.
        why: &'static str,
    },
    /// Decided per remote by the verifying read's first step: it rewrites where a remote
    /// resolves, and §45.3(a) classifies the resolved URL.
    PerRemote,
    /// Cannot move what any intent writes beyond its declared effect.
    Inert {
        /// The one-line reason.
        why: &'static str,
    },
}

const GC_NEVER_RUNS: KeyClass = KeyClass::Inert {
    why: "read only by gc, which gc.auto=0, gc.autoDetach=false and maintenance.auto=false keep \
          from running",
};
const MAINTENANCE_NEVER_RUNS: KeyClass = KeyClass::Inert {
    why: "read only by maintenance, which maintenance.auto=false keeps from running",
};
const TRANSPORT_TUNING: KeyClass = KeyClass::Inert {
    why: "transport behaviour; the read's effect is objects-only and a clone's a new repository \
          whatever the transport does",
};
const NO_SIGNING: KeyClass = KeyClass::Inert {
    why: "read only when signing or verifying; no Lane-0 intent signs",
};
const NO_COMMIT_OR_TAG: KeyClass = KeyClass::Inert {
    why: "read only when a commit or tag is written; no Lane-0 intent writes one",
};
const PUSH_ONLY: KeyClass = KeyClass::Inert {
    why: "read only by push; no intent pushes (U1)",
};
const NO_RECURSION: KeyClass = KeyClass::Inert {
    why: "read only when a command recurses into submodules; every fetch carries \
          --no-recurse-submodules and a clone passes no --recurse-submodules",
};
const NEW_REPOSITORY_ONLY: KeyClass = KeyClass::Inert {
    why: "shapes only the repository a clone creates, which is its declared effect; a hook it \
          copies never runs under core.hooksPath",
};
const BUNDLE_LIST: KeyClass = KeyClass::Inert {
    why: "a bundle list's own keys, read only through fetch.bundleURI or transfer.bundleURI, both \
          pinned off",
};
const REFUSES_ONLY: KeyClass = KeyClass::Inert {
    why: "checks what arrives and can only refuse it",
};
const HELPERS_PINNED: KeyClass = KeyClass::Pinned {
    by: "-c credential.helper=<the write path's own channel>, which resets every helper read \
         before it",
};
const PRUNE_PINNED: KeyClass = KeyClass::Pinned {
    by: "--no-prune on the objects step",
};

/// §47.9 B's classification table. A key the gate's git lists in an audited namespace and this
/// table omits fails `git_config_tripwire`.
pub const CONFIG_KEY_CLASSES: &[(&str, KeyClass)] = &[
    ("bundle.*", BUNDLE_LIST),
    ("bundle.<id>.*", BUNDLE_LIST),
    ("bundle.<id>.uri", BUNDLE_LIST),
    ("bundle.heuristic", BUNDLE_LIST),
    ("bundle.mode", BUNDLE_LIST),
    ("bundle.version", BUNDLE_LIST),
    ("clone.defaultRemoteName", NEW_REPOSITORY_ONLY),
    ("clone.filterSubmodules", NO_RECURSION),
    ("clone.rejectShallow", REFUSES_ONLY),
    ("core.askPass", KeyClass::Pinned { by: "-c core.askPass=" }),
    ("core.fsmonitor", KeyClass::Pinned { by: "-c core.fsmonitor=false" }),
    (
        "core.gitProxy",
        KeyClass::Inert {
            why: "runs only for the git:// transport, which GIT_ALLOW_PROTOCOL refuses",
        },
    ),
    (
        "core.hooksPath",
        KeyClass::Pinned {
            by: "-c core.hooksPath=<the core's empty hooks directory>",
        },
    ),
    (
        "core.logAllRefUpdates",
        KeyClass::Inert {
            why: "a reflog is written only beside a ref write: a clone's in its new repository, and \
                  the verifying read writes no ref",
        },
    ),
    (
        "core.sshCommand",
        KeyClass::Inert {
            why: "the user's own ssh transport, admitted for an ssh remote by PA1; a clone refuses \
                  ssh through GIT_ALLOW_PROTOCOL",
        },
    ),
    ("credential.<url>.*", HELPERS_PINNED),
    ("credential.helper", HELPERS_PINNED),
    (
        "credential.interactive",
        KeyClass::Inert {
            why: "GIT_TERMINAL_PROMPT=0 is set on every child",
        },
    ),
    ("credential.protectProtocol", REFUSES_ONLY),
    ("credential.sanitizePrompt", REFUSES_ONLY),
    ("credential.useHttpPath", HELPERS_PINNED),
    ("credential.username", HELPERS_PINNED),
    (
        "fetch.all",
        KeyClass::Inert {
            why: "the objects step names its one remote and passes no --all",
        },
    ),
    (
        "fetch.bundleCreationToken",
        KeyClass::Pinned {
            by: "-c fetch.bundleURI= (read and written only beside a bundle URI)",
        },
    ),
    ("fetch.bundleURI", KeyClass::Pinned { by: "-c fetch.bundleURI=" }),
    ("fetch.fsck.<msg-id>", REFUSES_ONLY),
    ("fetch.fsck.skipList", REFUSES_ONLY),
    ("fetch.fsckObjects", REFUSES_ONLY),
    ("fetch.negotiationAlgorithm", TRANSPORT_TUNING),
    ("fetch.output", TRANSPORT_TUNING),
    ("fetch.parallel", TRANSPORT_TUNING),
    ("fetch.prune", PRUNE_PINNED),
    ("fetch.pruneTags", PRUNE_PINNED),
    (
        "fetch.recurseSubmodules",
        KeyClass::Pinned {
            by: "--no-recurse-submodules on the objects step",
        },
    ),
    ("fetch.showForcedUpdates", TRANSPORT_TUNING),
    ("fetch.unpackLimit", TRANSPORT_TUNING),
    (
        "fetch.writeCommitGraph",
        KeyClass::Pinned {
            by: "--no-write-commit-graph on the objects step",
        },
    ),
    (
        "filter.<driver>.clean",
        KeyClass::Pinned {
            by: "filter.<driver>.clean= for every driver the enumeration found (Clone)",
        },
    ),
    (
        "filter.<driver>.smudge",
        KeyClass::Pinned {
            by: "filter.<driver>.smudge= for every driver the enumeration found (Clone)",
        },
    ),
    ("gc.<pattern>.reflogExpire", GC_NEVER_RUNS),
    ("gc.<pattern>.reflogExpireUnreachable", GC_NEVER_RUNS),
    ("gc.aggressiveDepth", GC_NEVER_RUNS),
    ("gc.aggressiveWindow", GC_NEVER_RUNS),
    ("gc.auto", KeyClass::Pinned { by: "-c gc.auto=0" }),
    ("gc.autoDetach", KeyClass::Pinned { by: "-c gc.autoDetach=false" }),
    ("gc.autoPackLimit", GC_NEVER_RUNS),
    ("gc.bigPackThreshold", GC_NEVER_RUNS),
    ("gc.cruftPacks", GC_NEVER_RUNS),
    ("gc.logExpiry", GC_NEVER_RUNS),
    ("gc.maxCruftSize", GC_NEVER_RUNS),
    ("gc.packRefs", GC_NEVER_RUNS),
    ("gc.pruneExpire", GC_NEVER_RUNS),
    ("gc.recentObjectsHook", GC_NEVER_RUNS),
    ("gc.reflogExpire", GC_NEVER_RUNS),
    ("gc.reflogExpireUnreachable", GC_NEVER_RUNS),
    ("gc.repackFilter", GC_NEVER_RUNS),
    ("gc.repackFilterTo", GC_NEVER_RUNS),
    ("gc.rerereResolved", GC_NEVER_RUNS),
    ("gc.rerereUnresolved", GC_NEVER_RUNS),
    ("gc.worktreePruneExpire", GC_NEVER_RUNS),
    ("gc.writeCommitGraph", GC_NEVER_RUNS),
    ("gpg.<format>.program", NO_SIGNING),
    ("gpg.format", NO_SIGNING),
    ("gpg.minTrustLevel", NO_SIGNING),
    ("gpg.program", NO_SIGNING),
    ("gpg.ssh.allowedSignersFile", NO_SIGNING),
    ("gpg.ssh.defaultKeyCommand", NO_SIGNING),
    ("gpg.ssh.revocationFile", NO_SIGNING),
    ("http.<url>.*", TRANSPORT_TUNING),
    ("http.allowNTLMAuth", TRANSPORT_TUNING),
    (
        "http.cookieFile",
        KeyClass::Inert {
            why: "the user's own cookie jar, outside every repository",
        },
    ),
    ("http.curloptResolve", TRANSPORT_TUNING),
    ("http.delegation", TRANSPORT_TUNING),
    ("http.emptyAuth", TRANSPORT_TUNING),
    ("http.extraHeader", TRANSPORT_TUNING),
    ("http.followRedirects", TRANSPORT_TUNING),
    ("http.keepAliveCount", TRANSPORT_TUNING),
    ("http.keepAliveIdle", TRANSPORT_TUNING),
    ("http.keepAliveInterval", TRANSPORT_TUNING),
    ("http.lowSpeedLimit", TRANSPORT_TUNING),
    ("http.lowSpeedTime", TRANSPORT_TUNING),
    ("http.maxRequests", TRANSPORT_TUNING),
    ("http.maxRetries", TRANSPORT_TUNING),
    ("http.maxRetryTime", TRANSPORT_TUNING),
    ("http.minSessions", TRANSPORT_TUNING),
    ("http.noEPSV", TRANSPORT_TUNING),
    ("http.pinnedPubkey", TRANSPORT_TUNING),
    ("http.postBuffer", TRANSPORT_TUNING),
    ("http.proactiveAuth", TRANSPORT_TUNING),
    ("http.proxy", TRANSPORT_TUNING),
    ("http.proxyAuthMethod", TRANSPORT_TUNING),
    ("http.proxySSLCAInfo", TRANSPORT_TUNING),
    ("http.proxySSLCert", TRANSPORT_TUNING),
    ("http.proxySSLCertPasswordProtected", TRANSPORT_TUNING),
    ("http.proxySSLKey", TRANSPORT_TUNING),
    ("http.retryAfter", TRANSPORT_TUNING),
    (
        "http.saveCookies",
        KeyClass::Inert {
            why: "writes the user's own cookie jar, outside every repository",
        },
    ),
    ("http.schannelCheckRevoke", TRANSPORT_TUNING),
    ("http.schannelUseSSLCAInfo", TRANSPORT_TUNING),
    ("http.sslAutoClientCert", TRANSPORT_TUNING),
    ("http.sslBackend", TRANSPORT_TUNING),
    ("http.sslCAInfo", TRANSPORT_TUNING),
    ("http.sslCAPath", TRANSPORT_TUNING),
    ("http.sslCert", TRANSPORT_TUNING),
    ("http.sslCertPasswordProtected", TRANSPORT_TUNING),
    ("http.sslCertType", TRANSPORT_TUNING),
    ("http.sslCipherList", TRANSPORT_TUNING),
    ("http.sslKey", TRANSPORT_TUNING),
    ("http.sslKeyType", TRANSPORT_TUNING),
    ("http.sslTry", TRANSPORT_TUNING),
    ("http.sslVerify", TRANSPORT_TUNING),
    ("http.sslVersion", TRANSPORT_TUNING),
    ("http.userAgent", TRANSPORT_TUNING),
    ("http.version", TRANSPORT_TUNING),
    (
        "include.path",
        KeyClass::Inert {
            why: "a config route: every key it includes is classified on its own",
        },
    ),
    (
        "includeIf.<condition>.path",
        KeyClass::Inert {
            why: "a config route: every key it includes is classified on its own",
        },
    ),
    ("init.defaultBranch", NEW_REPOSITORY_ONLY),
    ("init.defaultObjectFormat", NEW_REPOSITORY_ONLY),
    ("init.defaultRefFormat", NEW_REPOSITORY_ONLY),
    ("init.defaultSubmodulePathConfig", NEW_REPOSITORY_ONLY),
    ("init.templateDir", NEW_REPOSITORY_ONLY),
    ("maintenance.<task>.enabled", MAINTENANCE_NEVER_RUNS),
    ("maintenance.<task>.schedule", MAINTENANCE_NEVER_RUNS),
    ("maintenance.auto", KeyClass::Pinned { by: "-c maintenance.auto=false" }),
    ("maintenance.autoDetach", MAINTENANCE_NEVER_RUNS),
    ("maintenance.commit-graph.auto", MAINTENANCE_NEVER_RUNS),
    ("maintenance.geometric-repack.auto", MAINTENANCE_NEVER_RUNS),
    ("maintenance.geometric-repack.splitFactor", MAINTENANCE_NEVER_RUNS),
    ("maintenance.incremental-repack.auto", MAINTENANCE_NEVER_RUNS),
    ("maintenance.loose-objects.auto", MAINTENANCE_NEVER_RUNS),
    ("maintenance.loose-objects.batchSize", MAINTENANCE_NEVER_RUNS),
    ("maintenance.reflog-expire.auto", MAINTENANCE_NEVER_RUNS),
    ("maintenance.rerere-gc.auto", MAINTENANCE_NEVER_RUNS),
    ("maintenance.strategy", MAINTENANCE_NEVER_RUNS),
    ("maintenance.worktree-prune.auto", MAINTENANCE_NEVER_RUNS),
    (
        "protocol.<name>.allow",
        KeyClass::Pinned {
            by: "GIT_ALLOW_PROTOCOL, set per intent after the scrub (§47 M4)",
        },
    ),
    (
        "protocol.allow",
        KeyClass::Pinned {
            by: "GIT_ALLOW_PROTOCOL, set per intent after the scrub (§47 M4)",
        },
    ),
    ("protocol.version", TRANSPORT_TUNING),
    (
        "remote.<name>.fetch",
        KeyClass::Pinned {
            by: "--refmap= and --stdin on the objects step: no configured refspec is used",
        },
    ),
    (
        "remote.<name>.followRemoteHEAD",
        KeyClass::Pinned {
            by: "--refmap= on the objects step: no tracking ref moves, remote HEAD included (§47 M1)",
        },
    ),
    ("remote.<name>.mirror", PUSH_ONLY),
    ("remote.<name>.negotiationInclude", TRANSPORT_TUNING),
    ("remote.<name>.negotiationRestrict", TRANSPORT_TUNING),
    (
        "remote.<name>.partialclonefilter",
        KeyClass::Inert {
            why: "can only fetch fewer objects, and the read re-checks every tip it fetched",
        },
    ),
    (
        "remote.<name>.promisor",
        KeyClass::Inert {
            why: "can only fetch fewer objects, and the read re-checks every tip it fetched",
        },
    ),
    ("remote.<name>.proxy", TRANSPORT_TUNING),
    ("remote.<name>.proxyAuthMethod", TRANSPORT_TUNING),
    ("remote.<name>.prune", PRUNE_PINNED),
    ("remote.<name>.pruneTags", PRUNE_PINNED),
    ("remote.<name>.push", PUSH_ONLY),
    ("remote.<name>.pushurl", PUSH_ONLY),
    ("remote.<name>.receivepack", PUSH_ONLY),
    ("remote.<name>.serverOption", TRANSPORT_TUNING),
    (
        "remote.<name>.skipDefaultUpdate",
        KeyClass::Inert {
            why: "read only by fetch --all and remote update; the objects step names its remote",
        },
    ),
    (
        "remote.<name>.skipFetchAll",
        KeyClass::Inert {
            why: "read only by fetch --all and remote update; the objects step names its remote",
        },
    ),
    ("remote.<name>.tagOpt", KeyClass::Pinned { by: "--no-tags on the objects step" }),
    (
        "remote.<name>.uploadpack",
        KeyClass::Inert {
            why: "runs on the remote side of ssh; locally only for file://, which \
                  GIT_ALLOW_PROTOCOL refuses",
        },
    ),
    ("remote.<name>.url", KeyClass::PerRemote),
    ("remote.<name>.vcs", KeyClass::PerRemote),
    ("remote.pushDefault", PUSH_ONLY),
    ("submodule.<name>.active", NO_RECURSION),
    ("submodule.<name>.branch", NO_RECURSION),
    (
        "submodule.<name>.fetchRecurseSubmodules",
        KeyClass::Pinned {
            by: "--no-recurse-submodules on the objects step",
        },
    ),
    ("submodule.<name>.gitdir", NO_RECURSION),
    ("submodule.<name>.ignore", NO_RECURSION),
    ("submodule.<name>.update", NO_RECURSION),
    ("submodule.<name>.url", NO_RECURSION),
    ("submodule.active", NO_RECURSION),
    ("submodule.alternateErrorStrategy", NO_RECURSION),
    ("submodule.alternateLocation", NO_RECURSION),
    ("submodule.fetchJobs", NO_RECURSION),
    ("submodule.propagateBranches", NO_RECURSION),
    (
        "submodule.recurse",
        KeyClass::Pinned {
            by: "--no-recurse-submodules on the objects step",
        },
    ),
    ("tag.forceSignAnnotated", NO_COMMIT_OR_TAG),
    ("tag.gpgSign", NO_COMMIT_OR_TAG),
    (
        "tag.sort",
        KeyClass::Inert {
            why: "orders a listing; writes nothing",
        },
    ),
    ("transfer.advertiseObjectInfo", TRANSPORT_TUNING),
    ("transfer.advertiseSID", TRANSPORT_TUNING),
    (
        "transfer.bundleURI",
        KeyClass::Pinned {
            by: "-c transfer.bundleURI=false",
        },
    ),
    ("transfer.credentialsInUrl", REFUSES_ONLY),
    ("transfer.fsckObjects", REFUSES_ONLY),
    (
        "transfer.hideRefs",
        KeyClass::Inert {
            why: "hides refs a server advertises; can only make a remote cover less",
        },
    ),
    ("transfer.unpackLimit", TRANSPORT_TUNING),
    ("url.<base>.insteadOf", KeyClass::PerRemote),
    ("url.<base>.pushInsteadOf", PUSH_ONLY),
    ("user.email", NO_COMMIT_OR_TAG),
    ("user.name", NO_COMMIT_OR_TAG),
    ("user.signingKey", NO_SIGNING),
    ("user.useConfigOnly", NO_COMMIT_OR_TAG),
];
