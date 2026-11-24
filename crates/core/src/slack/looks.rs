//! The icon and the name the secretary's posts on Slack carry.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use crate::persona::{self, Gender};
use crate::store::Store;

/// Where Slack fetches the brand's icons from: the public repository.
const BRAND: &str = "https://raw.githubusercontent.com/lambdalisue/famulen/main/brand";

pub(super) fn url(gender: Gender) -> String {
    format!("{BRAND}/{}/avatar-512.png", gender.as_str())
}

/// How a post shows the secretary.
#[derive(Debug, PartialEq)]
pub struct Look {
    pub icon: String,
    pub name: String,
}

/// The secretary's look, and the connections whose workspace let the bot post
/// but not with a look of its own. Clones share both.
#[derive(Clone)]
pub struct Looks {
    store: Store,
    refused: Arc<Mutex<BTreeSet<String>>>,
}

impl Looks {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            refused: Arc::default(),
        }
    }

    /// The look to post with now; none where the workspace refused one.
    pub fn of(&self, connection: &str) -> Option<Look> {
        if self.refused().contains(connection) {
            return None;
        }
        match Gender::load(&self.store) {
            Ok(gender) => Some(Look {
                icon: url(gender),
                name: persona::name(&self.store),
            }),
            // A message is never held back for its icon.
            Err(error) => {
                tracing::warn!(%error, "could not read which icon the secretary has");
                None
            }
        }
    }

    /// Post without a look there until the app is installed again or
    /// Famulen starts again.
    pub fn refuse(&self, connection: &str) {
        self.refused().insert(connection.to_owned());
    }

    /// The app was installed there again, perhaps with the scope.
    pub fn installed(&self, connection: &str) {
        self.refused().remove(connection);
    }

    fn refused(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.refused.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
