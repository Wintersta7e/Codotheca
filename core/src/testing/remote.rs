//! A [`RemoteVerifier`] double (§45.14): the one seam of the analyser a test may stand in for.
//!
//! It answers each remote with a scripted reading — a fixture upstream's advertisement whose
//! objects are already local, or one of the did-not-answer classes — and records every remote it
//! was asked about, in order, so a test can count verifier calls. **An unscripted remote did not
//! answer**: a double that answered *covered* by default would teach a deletion gate that every
//! commit is pushed.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::analyser::remote::{DidNotAnswer, RemoteReading, RemoteVerifier};
use crate::git::{JobContext, RepoHandle};

/// The scripted verifier.
#[derive(Debug, Default)]
pub struct FixtureRemoteVerifier {
    readings: Mutex<BTreeMap<String, RemoteReading>>,
    calls: Mutex<Vec<String>>,
}

impl FixtureRemoteVerifier {
    /// A verifier with nothing scripted: every remote did not answer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer `remote` with `reading` from now on.
    pub fn script(&self, remote: &str, reading: RemoteReading) -> &Self {
        self.readings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(remote.to_owned(), reading);
        self
    }

    /// `remote` answered, advertising `present` — object ids already in the local store.
    pub fn answering(&self, remote: &str, present: &[String]) -> &Self {
        self.script(
            remote,
            RemoteReading::Answered {
                present: present.to_vec(),
                fetched: Vec::new(),
            },
        )
    }

    /// `remote` did not answer, for `why`.
    pub fn silent(&self, remote: &str, why: DidNotAnswer) -> &Self {
        self.script(remote, RemoteReading::DidNotAnswer(why))
    }

    /// Every remote the verifier was asked to read, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl RemoteVerifier for FixtureRemoteVerifier {
    fn read(&self, _repo: &RepoHandle, remote: &str, _ctx: &JobContext<'_>) -> RemoteReading {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(remote.to_owned());
        self.readings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(remote)
            .cloned()
            .unwrap_or(RemoteReading::DidNotAnswer(DidNotAnswer::Failed))
    }
}
