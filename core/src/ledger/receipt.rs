//! §38.2's receipts: a retained row's `meta` carries its own sentence — what, when, and the
//! project's name as of the write — so it renders without a join to a row since renamed, merged
//! or tombstoned.

use serde_json::{Map, Value};

use crate::clock::LocalInstant;

/// A retained row's `meta`, built one way for every kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    fields: Map<String, Value>,
}

impl Receipt {
    /// `what`, the local date of `at` as `when`, and `project_name` as it stands at the write.
    #[must_use]
    pub fn new(what: &str, at: LocalInstant, project_name: &str) -> Self {
        Self::day(at, project_name).extra("what", what)
    }

    /// A `debt_day`'s receipt: no `what`, because the day's `sources` and `closed` say what was
    /// done.
    #[must_use]
    pub fn day(at: LocalInstant, project_name: &str) -> Self {
        let mut fields = Map::new();
        fields.insert("when".to_owned(), Value::from(at.date()));
        fields.insert("project_name".to_owned(), Value::from(project_name));
        Self { fields }
    }

    /// A row of no project's — a group approval's: `what` and the local date of `at` as `when`,
    /// and no name.
    #[must_use]
    pub fn library(what: &str, at: LocalInstant) -> Self {
        let mut fields = Map::new();
        fields.insert("what".to_owned(), Value::from(what));
        fields.insert("when".to_owned(), Value::from(at.date()));
        Self { fields }
    }

    /// §38.2's optional `silent` key, written only when true: an absent key reads as not silent,
    /// and a restored row carries none.
    #[must_use]
    pub fn silent(mut self, silent: bool) -> Self {
        if silent {
            self.fields.insert("silent".to_owned(), Value::Bool(true));
        } else {
            self.fields.remove("silent");
        }
        self
    }

    /// One of the kind's own keys beside the receipt's.
    #[must_use]
    pub fn extra(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(key.to_owned(), value.into());
        self
    }

    /// The `meta` text the row stores.
    #[must_use]
    pub fn to_meta(&self) -> String {
        Value::Object(self.fields.clone()).to_string()
    }
}
