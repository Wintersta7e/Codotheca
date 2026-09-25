//! §21.9's closed outcome enum.
//!
//! `Throttled` and `Unauthorized` are the two phase 1 has no word for, and they are the two that
//! decide whether the app burns an account's allowance in a loop.

/// What one forge response means to the task that caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// A 2xx that delivered everything asked for: no further page follows.
    Done,
    /// A 304: the conditional request matched, and nothing changed since the last answer.
    NotModified,
    /// A 2xx whose page carried a cursor, promoted by [`SyncOutcome::with_next_page`].
    NextPage {
        /// The cursor the provider parsed out of the page, handed back to ask for the next one.
        cursor: String,
    },
    /// **`secondary` is this plan's, and it is a Rust-only widening of §21.9's `{ until }`.**
    ///
    /// §21.8 step 4 rules that a throttle *"is a **secondary** limit iff `retry-after` is present
    /// and `remaining > 0`"*, and §21.4 backs the two off **differently** — a secondary limit is
    /// floored at 60 s, doubled per `throttle_count` and capped at an hour, while a primary yield
    /// goes to `reset_at` exactly and increments nothing. `apply_outcome` therefore has to know
    /// which it was, and by then the headers are gone; recomputing it downstream would restate
    /// the classifier's decision without its inputs. **The wire is unaffected**: `SyncOutcomeKind`
    /// is a flat enum with one `throttled` variant.
    Throttled {
        /// The later of `retry-after` and `x-ratelimit-reset`, in Unix seconds on our clock;
        /// `now` when the server named neither.
        until: i64,
        /// A secondary limit — `retry-after` present and `remaining > 0` — rather than a
        /// primary yield.
        secondary: bool,
    },
    /// A 401, or a 403 with neither `retry-after` nor a spent `remaining`: the row goes `blocked`
    /// and is never retried on a backoff.
    Unauthorized {
        /// What the response lets us say about why.
        reason: UnauthorizedReason,
    },
    /// A 404: the resource is *unseen*, never *gone*, and no row is deleted for it.
    NotFound,
    /// Any other 4xx, or a status no other step of §21.8 claims: terminal, because a request
    /// this client formed wrongly will be formed wrongly again.
    Rejected {
        /// The HTTP status the forge answered.
        status: u16,
    },
    /// A 5xx, a transport failure, a provider call that failed before any response, or a task
    /// step that ended in a [`crate::sync::SyncError`]: retried on §21.4's transient backoff and
    /// `deferred` at the third.
    TransientFail {
        /// The transport's, the server's or the provider's own words, for the row and the log.
        reason: String,
    },
}

impl SyncOutcome {
    /// §21.8 step 9's second half, applied by the task that **parsed the page**.
    ///
    /// `classify` sees status, headers and body length; the next cursor is parsed out of the body
    /// by the provider, one layer above the transport this plan decorates. So the promotion
    /// happens where the cursor exists, in one function with one production caller, rather than
    /// as a `cursor` parameter the decorator could only ever pass `None` for.
    ///
    /// Only a `Done` promotes: a 304, a park or a refusal did not deliver a page, so there is no
    /// next one to ask for.
    #[must_use]
    pub fn with_next_page(self, cursor: Option<String>) -> Self {
        match (self, cursor) {
            (Self::Done, Some(cursor)) => Self::NextPage { cursor },
            (other, _) => other,
        }
    }

    /// The flat wire vocabulary §21.13 declares, which carries the variant and none of its data.
    #[must_use]
    pub const fn kind(&self) -> crate::protocol::SyncOutcomeKind {
        use crate::protocol::SyncOutcomeKind as K;
        match self {
            Self::Done => K::Done,
            Self::NotModified => K::NotModified,
            Self::NextPage { .. } => K::NextPage,
            Self::Throttled { .. } => K::Throttled,
            Self::Unauthorized { .. } => K::Unauthorized,
            Self::NotFound => K::NotFound,
            Self::Rejected { .. } => K::Rejected,
            Self::TransientFail { .. } => K::TransientFail,
        }
    }
}

/// Why a response was unauthorised, **in the vocabulary the response can actually distinguish**.
///
/// §21.8 names four reasons — `token_invalid`, `sso_required`, `missing_scope`, `revoked` — and
/// only the first two are observable. GitHub marks an SSO refusal with `x-github-sso` on the 403
/// (`core/src/accounts/commands.rs:113-129`), and **nothing separates a missing scope from a
/// revoked grant**: both arrive as a bare 403. p2-20 already recorded that and collapses them
/// (`core/src/accounts/commands.rs:136-143`, *"insufficient scope, or an OAuth app the org
/// restricts — and both are answered on the account surface"*).
///
/// So the third variant is `Forbidden`, which is §21.8's own table spelling at
/// `.dev/spec/21-remote-sync.md:203` (`Unauthorized{forbidden}`) and one of §21.13's four
/// `SyncNotice` variants. **A reason the classifier cannot observe is a fact invented at exactly
/// the point where the user is told what to do about it** — R100's rule, reached by not declaring
/// the state rather than by asserting it never occurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnauthorizedReason {
    /// The 401. The token does not authenticate.
    TokenInvalid,
    /// A 403 carrying `x-github-sso`: the grant is real and an organisation has not authorised it.
    SsoRequired,
    /// A 403 carrying neither a rate header nor an SSO header. A missing scope or a revoked
    /// grant, which this response cannot tell apart; both are fixed on the account surface.
    Forbidden,
}

impl UnauthorizedReason {
    /// Written into `sync_task_state.reason`.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::TokenInvalid => "token_invalid",
            Self::SsoRequired => "sso_required",
            Self::Forbidden => "forbidden",
        }
    }

    /// The same condition seen as a **command** error, in p2-20's vocabulary.
    ///
    /// §21.8: *"§20's `TOKEN_INVALID` and `SSO_REQUIRED` are `ErrorCode`s for a command; this is
    /// the same condition seen as state, and neither is derived from the other's spelling by
    /// hand"* (R24). This is the one mapping function, and its test reads p2-20's generated enum
    /// rather than restating either spelling.
    #[must_use]
    pub const fn error_code(self) -> crate::protocol::ErrorCode {
        match self {
            Self::SsoRequired => crate::protocol::ErrorCode::SsoRequired,
            // p2-20's own ruling: a 403 the SSO check declined is this token's identity being
            // refused for the call, and it is answered on the account surface.
            Self::TokenInvalid | Self::Forbidden => crate::protocol::ErrorCode::TokenInvalid,
        }
    }
}
