//! §24.1c: how a credential reaches git, and how it does not.
//!
//! **`credential.helper` takes exactly one explicit value and is never absent**, because absence
//! means inheritance: git would fall back to the user's configured helper, and a `repo`-scoped
//! token sitting in their OS credential manager would silently give a clone push rights the tier
//! design exists to withhold. The audit therefore **counts** the occurrences of
//! `-c credential.helper=` rather than searching for one — a test that only rejects a wrong value
//! passes on inheritance, which is the whole hazard.
//!
//! The four mechanisms §24.1c rejects, and why, so nobody reintroduces one:
//!
//! | Rejected | Why |
//! |---|---|
//! | URL userinfo, `https://<token>@host/…` | In argv, in git's own error output, and **persisted in the clone's `.git/config`** after the operation ends. The worst of the four |
//! | `-c http.extraHeader=Authorization: …` | In argv |
//! | `GIT_ASKPASS` / `SSH_ASKPASS` | Names a program, so the secret must still reach it — and setting either undoes §3.2's stripping for the whole child tree |
//! | Inheriting the user's helper | Borrows credentials this product was never granted |

use std::ffi::OsString;

/// The `-c credential.helper=` value for one invocation.
///
/// Renders **exactly one** `-c credential.helper=<value>` pair, always.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialChannel {
    /// No credential at all.
    ///
    /// §24.1c: *"A public clone carries no credential at all."* Anonymous HTTPS is the whole of
    /// the default tier's clone surface, so the `public`-tier token never reaches git. This is a
    /// complete behaviour rather than a placeholder — it is what the shipped default tier does.
    Anonymous,
}

impl CredentialChannel {
    /// A clone that authenticates with nothing.
    #[must_use]
    pub fn anonymous() -> CredentialChannel {
        CredentialChannel::Anonymous
    }

    /// The single `-c credential.helper=<value>` pair this channel renders.
    ///
    /// Returns the `-c` and its value as two elements, because argv is a list and a config option
    /// is two elements of it — joining them into one string is how a value with a space stops
    /// being one argument.
    #[must_use]
    pub fn helper_args(&self) -> Vec<OsString> {
        match self {
            CredentialChannel::Anonymous => {
                vec![OsString::from("-c"), OsString::from("credential.helper=")]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::CredentialChannel;

    #[test]
    fn an_anonymous_channel_renders_one_empty_helper_and_not_zero() {
        let argv = CredentialChannel::anonymous().helper_args();
        let rendered: Vec<String> = argv
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            vec!["-c", "credential.helper="],
            "an empty value is not the same as no value: absence is inheritance"
        );
    }
}
