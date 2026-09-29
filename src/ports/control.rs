//! The operations log: what the control port was asked to do and what it
//! answered, appended before anything runs and never rewritten. It is
//! what makes "an operation absent from the log never ran" a fact the
//! asker can rely on after a lost reply, whatever happened to the records
//! since.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "line", rename_all = "kebab-case")]
pub enum OpLine {
    /// The command was received and its records are being saved; the
    /// effect that launches anything runs after this line is on disk.
    Requested {
        op: String,
        kind: String,
        ids: Vec<String>,
        at: SystemTime,
    },
    /// The terminal reply, as the wire carried it.
    Replied {
        op: String,
        reply: String,
        at: SystemTime,
    },
}

impl OpLine {
    #[must_use]
    pub fn op(&self) -> &str {
        match self {
            Self::Requested { op, .. } | Self::Replied { op, .. } => op,
        }
    }
}

pub trait Operations: Send {
    /// Append one line and make sure it is on disk before returning.
    fn append(&mut self, line: &OpLine) -> std::io::Result<()>;
    /// Every line about `op`, oldest first.
    fn find(&self, op: &str) -> Vec<OpLine>;
}
