//! What moves the remote port's revision: a fingerprint of every
//! exported read model, taken after a pass and compared with the last
//! one. The diff is the gate rather than the effect list, because a
//! save is not a reliable sign of a visible change (a set renames
//! through `SaveViews`, a trust question or a quiet Codex pane changes
//! a card with no save at all) and an effect is not always a change.
//! Time-derived fields are held still before hashing, so a pane that
//! has merely aged does not count; the quiet state the card shows is
//! what the diff sees.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::SystemTime;

use super::AppCore;
use super::model::{ProjectId, SpaceId};

/// One resource a client holds a revision for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKind {
    Project,
    Space,
    Set,
    Session,
    /// The collections: emitted when the member id set moves, answered
    /// by refetching the collection whole.
    Projects,
    Spaces,
    /// A space's sets; the id is the space's.
    Sets,
    /// A project's sessions; the id is the project's.
    Sessions,
    Waiting,
}

impl ResourceKind {
    /// The `kind` string on the wire.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Space => "space",
            Self::Set => "set",
            Self::Session => "session",
            Self::Projects => "projects",
            Self::Spaces => "spaces",
            Self::Sets => "sets",
            Self::Sessions => "sessions",
            Self::Waiting => "waiting",
        }
    }
}

/// A resource by kind and id; a global collection's id is empty.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Resource {
    pub kind: ResourceKind,
    pub id: String,
}

/// One difference between two passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub resource: Resource,
    /// The resource is gone rather than different.
    pub removed: bool,
}

/// The read models at one moment, each reduced to a hash. A change
/// detector, not a commitment: `SipHash` over the view's fields is
/// cheap enough to run on every pass that dispatched something.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fingerprints {
    items: BTreeMap<Resource, u64>,
}

impl Fingerprints {
    /// Every exported read model of `core`, hashed.
    #[must_use]
    pub fn of(core: &AppCore, now: SystemTime) -> Self {
        let mut items = BTreeMap::new();
        let mut put = |kind: ResourceKind, id: &str, hash: u64| {
            items.insert(
                Resource {
                    kind,
                    id: id.to_owned(),
                },
                hash,
            );
        };
        let projects = core.project_views(None);
        for p in &projects {
            put(ResourceKind::Project, &p.id, hash_of(p));
        }
        put(
            ResourceKind::Projects,
            "",
            ids_hash(projects.iter().map(|p| &p.id)),
        );
        let spaces = core.space_views();
        for s in &spaces {
            put(ResourceKind::Space, &s.id, hash_of(s));
            let Ok(id) = uuid::Uuid::parse_str(&s.id) else {
                continue;
            };
            let sets = core.set_views(SpaceId(id));
            for set in &sets {
                put(ResourceKind::Set, &set.id, hash_of(set));
            }
            put(
                ResourceKind::Sets,
                &s.id,
                ids_hash(sets.iter().map(|x| &x.id)),
            );
        }
        put(
            ResourceKind::Spaces,
            "",
            ids_hash(spaces.iter().map(|s| &s.id)),
        );
        for p in &projects {
            let Ok(id) = uuid::Uuid::parse_str(&p.id) else {
                continue;
            };
            let Some(w) = core.workspace(ProjectId(id)) else {
                continue;
            };
            let mut ids = Vec::new();
            for s in &w.sessions {
                if let Some(mut view) = core.session_view(s.id, now) {
                    // Age is not a change; the card's word is.
                    view.quiet_secs = None;
                    put(ResourceKind::Session, &view.id, hash_of(&view));
                    ids.push(view.id);
                }
            }
            put(ResourceKind::Sessions, &p.id, ids_hash(ids.iter()));
        }
        let waiting = core.waiting_views(now);
        put(
            ResourceKind::Waiting,
            "",
            ids_hash(waiting.iter().map(|s| &s.id)),
        );
        Self { items }
    }

    /// What moved between `self` and `next`: one `changed` per resource
    /// whose hash differs or that is new, one `removed` per resource
    /// gone, in a fixed order.
    #[must_use]
    pub fn changes(&self, next: &Self) -> Vec<Change> {
        let mut out = Vec::new();
        for (resource, hash) in &next.items {
            if self.items.get(resource) != Some(hash) {
                out.push(Change {
                    resource: resource.clone(),
                    removed: false,
                });
            }
        }
        for resource in self.items.keys() {
            if !next.items.contains_key(resource) {
                out.push(Change {
                    resource: resource.clone(),
                    removed: true,
                });
            }
        }
        out
    }

    /// How many resources are fingerprinted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn hash_of<T: Hash>(view: &T) -> u64 {
    let mut h = DefaultHasher::new();
    view.hash(&mut h);
    h.finish()
}

/// The membership of a collection: its ids in order.
fn ids_hash<'a>(ids: impl Iterator<Item = &'a String>) -> u64 {
    let mut h = DefaultHasher::new();
    for id in ids {
        id.hash(&mut h);
    }
    h.finish()
}
