//! Workspace member index: owner -> member references.

use hashbrown::{HashMap, HashSet};
use std::sync::Arc;

use smol_str::SmolStr;

use crate::semantic_db::SemanticDatabase;
use crate::semantic_db::def::{MemberRef, OwnerId, SemanticId};
use crate::semantic_db::exports::FileExports;
use crate::semantic_db::query::file_matches_workspace_id;
use crate::{FileId, WorkspaceId};

// Phase 2: workspace member association (after full analysis)
// ──────────────────────────────────────────────

/// Workspace-level member index: owner -> member references.
///
/// Each owner bucket keeps the declarations in source order, so overloads stay
/// stable, and a per-file key list makes remove/add O(file contribution).
///
/// Aggregate lists (`all` / `by_name`) are stored as `Arc<[MemberRef]>` and
/// rebuilt once per owner after a file add/remove batch; queries clone the
/// `Arc` instead of re-collecting the bucket on every call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OwnerMembers {
    /// The bucket in declaration order; the only persistent copy of the members.
    all: Arc<[MemberRef]>,
    /// Per-name lists, only when the bucket holds more than one name. A
    /// single-name bucket (the common case) answers `named` from `all`.
    by_name: HashMap<SmolStr, Arc<[MemberRef]>>,
    /// Working copy between the first `insert`/`remove` and the next
    /// `rebuild_caches`; `None` while the bucket is frozen.
    open: Option<OpenBucket>,
}

/// Mutable form of a bucket while a file add/remove batch is applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct OpenBucket {
    members: Vec<MemberRef>,
    /// Ids of `members`, kept only once the bucket outgrows a linear scan.
    ids: Option<HashSet<SemanticId>>,
}

/// Bucket size from which `OpenBucket` tracks member ids in a set instead of scanning.
const LINEAR_SCAN_LIMIT: usize = 16;

impl OpenBucket {
    fn contains(&self, member_id: &SemanticId) -> bool {
        match &self.ids {
            Some(ids) => ids.contains(member_id),
            None => self.members.iter().any(|member| &member.id == member_id),
        }
    }
}

impl OwnerMembers {
    fn open(&mut self) -> &mut OpenBucket {
        self.open.get_or_insert_with(|| OpenBucket {
            members: self.all.to_vec(),
            ids: None,
        })
    }

    fn insert(&mut self, member: MemberRef) -> bool {
        let known = match &self.open {
            Some(open) => open.contains(&member.id),
            None => self.all.iter().any(|existing| existing.id == member.id),
        };
        if known {
            return false;
        }
        let open = self.open();
        if let Some(ids) = &mut open.ids {
            ids.insert(member.id.clone());
        }
        open.members.push(member);
        if open.ids.is_none() && open.members.len() > LINEAR_SCAN_LIMIT {
            open.ids = Some(open.members.iter().map(|m| m.id.clone()).collect());
        }
        true
    }

    fn remove(&mut self, member_id: &SemanticId) -> bool {
        if self.open.is_none() && !self.all.iter().any(|member| &member.id == member_id) {
            return false;
        }
        let open = self.open();
        if !open.contains(member_id) {
            return false;
        }
        open.members.retain(|member| &member.id != member_id);
        if let Some(ids) = &mut open.ids {
            ids.remove(member_id);
        }
        true
    }

    fn is_empty(&self) -> bool {
        match &self.open {
            Some(open) => open.members.is_empty(),
            None => self.all.is_empty(),
        }
    }

    /// Freeze the bucket and rebuild the shared aggregate lists after it changed.
    fn rebuild_caches(&mut self) {
        if let Some(open) = self.open.take() {
            self.all = Arc::from(open.members);
        }

        let single_name = self
            .all
            .first()
            .is_none_or(|first| self.all.iter().all(|member| member.name == first.name));
        self.by_name = HashMap::new();
        if single_name {
            return;
        }
        let mut names: HashMap<&SmolStr, Vec<MemberRef>> = HashMap::new();
        for member in self.all.iter() {
            names.entry(&member.name).or_default().push(member.clone());
        }
        self.by_name = names
            .into_iter()
            .map(|(name, members)| (name.clone(), Arc::from(members)))
            .collect();
    }

    /// Distinct member names of the bucket.
    fn names(&self) -> impl Iterator<Item = &SmolStr> {
        let single = self
            .by_name
            .is_empty()
            .then(|| self.all.first().map(|member| &member.name))
            .flatten();
        single.into_iter().chain(self.by_name.keys())
    }

    fn all(&self) -> Arc<[MemberRef]> {
        Arc::clone(&self.all)
    }

    fn named(&self, name: &str) -> Option<Arc<[MemberRef]>> {
        if self.by_name.is_empty() {
            return self
                .all
                .first()
                .is_some_and(|member| member.name == name)
                .then(|| Arc::clone(&self.all));
        }
        self.by_name.get(name).cloned()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceMemberIndex {
    owners: HashMap<SemanticId, OwnerMembers>,
    /// Canonical `OwnerId` buckets. Module export / require alias members attach
    /// here so `require("mod").extra` can see members contributed by other files.
    owners_by_id: HashMap<OwnerId, OwnerMembers>,
    by_file: HashMap<FileId, Vec<(SemanticId, SemanticId)>>,
    by_file_owner_id: HashMap<FileId, Vec<(OwnerId, SemanticId)>>,
}

impl WorkspaceMemberIndex {
    fn rebuild_owner_caches<T: Eq + std::hash::Hash>(
        owners: &mut HashMap<T, OwnerMembers>,
        touched: HashSet<T>,
    ) {
        for owner in touched {
            let is_empty = owners.get(&owner).is_none_or(OwnerMembers::is_empty);
            if is_empty {
                owners.remove(&owner);
            } else if let Some(bucket) = owners.get_mut(&owner) {
                bucket.rebuild_caches();
            }
        }
    }

    pub(crate) fn members_of_owner(&self, owner: &SemanticId) -> Option<Arc<[MemberRef]>> {
        self.owners.get(owner).map(OwnerMembers::all)
    }

    pub(crate) fn members_of_owner_named(
        &self,
        owner: &SemanticId,
        name: &str,
    ) -> Option<Arc<[MemberRef]>> {
        self.owners.get(owner).and_then(|bucket| bucket.named(name))
    }

    pub(crate) fn members_of_owner_id(&self, owner: &OwnerId) -> Option<Arc<[MemberRef]>> {
        self.owners_by_id.get(owner).map(OwnerMembers::all)
    }

    pub(crate) fn members_of_owner_id_named(
        &self,
        owner: &OwnerId,
        name: &str,
    ) -> Option<Arc<[MemberRef]>> {
        self.owners_by_id
            .get(owner)
            .and_then(|bucket| bucket.named(name))
    }

    pub(crate) fn remove_file(&mut self, file_id: FileId) {
        let mut touched_raw: HashSet<SemanticId> = HashSet::new();
        if let Some(keys) = self.by_file.remove(&file_id) {
            for (owner, member_id) in keys {
                if let Some(bucket) = self.owners.get_mut(&owner)
                    && bucket.remove(&member_id)
                {
                    touched_raw.insert(owner);
                }
            }
        }
        Self::rebuild_owner_caches(&mut self.owners, touched_raw);

        let mut touched_canonical: HashSet<OwnerId> = HashSet::new();
        if let Some(keys) = self.by_file_owner_id.remove(&file_id) {
            for (owner, member_id) in keys {
                if let Some(bucket) = self.owners_by_id.get_mut(&owner)
                    && bucket.remove(&member_id)
                {
                    touched_canonical.insert(owner);
                }
            }
        }
        Self::rebuild_owner_caches(&mut self.owners_by_id, touched_canonical);
    }

    /// Insert one file's member contributions without rebuilding aggregate caches.
    ///
    /// Returns the `by_file` key lists; callers that batch many files (full
    /// workspace build) call [`Self::rebuild_touched_caches`] once at the end.
    fn add_file_inner(
        &mut self,
        exports: &FileExports,
        touched_raw: &mut HashSet<SemanticId>,
        touched_canonical: &mut HashSet<OwnerId>,
    ) -> (Vec<(SemanticId, SemanticId)>, Vec<(OwnerId, SemanticId)>) {
        let mut keys = Vec::new();
        let mut canonical_keys = Vec::new();
        for member in &exports.members {
            let owner = member.owner.clone();
            let member_id = member.member.clone();
            let name: SmolStr = member.key.to_path().into();
            let member_ref = MemberRef {
                file_id: member.file_id,
                id: member_id.clone(),
                name: name.clone(),
            };

            let bucket = self.owners.entry(owner.clone()).or_default();
            if bucket.insert(member_ref.clone()) {
                keys.push((owner.clone(), member_id.clone()));
                touched_raw.insert(owner);
            }

            let canonical_owner = member.owner_id.clone();
            let canonical_bucket = self
                .owners_by_id
                .entry(canonical_owner.clone())
                .or_default();
            if canonical_bucket.insert(member_ref) {
                canonical_keys.push((canonical_owner.clone(), member_id));
                touched_canonical.insert(canonical_owner);
            }
        }
        (keys, canonical_keys)
    }

    fn rebuild_touched_caches(
        &mut self,
        touched_raw: HashSet<SemanticId>,
        touched_canonical: HashSet<OwnerId>,
    ) {
        Self::rebuild_owner_caches(&mut self.owners, touched_raw);
        Self::rebuild_owner_caches(&mut self.owners_by_id, touched_canonical);
    }

    /// Raw owner identities present in this workspace bucket.
    fn owner_ids(&self) -> impl Iterator<Item = SemanticId> + '_ {
        self.owners.keys().cloned()
    }

    /// Raw `(owner, name)` keys present in this workspace bucket.
    fn owner_name_keys(&self) -> impl Iterator<Item = (SemanticId, SmolStr)> + '_ {
        self.owners.iter().flat_map(|(owner, bucket)| {
            bucket
                .names()
                .map(move |name| (owner.clone(), name.clone()))
        })
    }

    pub(crate) fn add_file(&mut self, file_id: FileId, exports: &FileExports) {
        let mut touched_raw: HashSet<SemanticId> = HashSet::new();
        let mut touched_canonical: HashSet<OwnerId> = HashSet::new();
        let (keys, canonical_keys) =
            self.add_file_inner(exports, &mut touched_raw, &mut touched_canonical);
        self.rebuild_touched_caches(touched_raw, touched_canonical);
        self.by_file.insert(file_id, keys);
        self.by_file_owner_id.insert(file_id, canonical_keys);
    }
}

/// Member index scoped to a single workspace.
pub(crate) fn build_workspace_member_index(
    db: &SemanticDatabase,
    ws_id: WorkspaceId,
) -> WorkspaceMemberIndex {
    #[cfg(test)]
    db.rebuild_metrics
        .full_index_source_scans
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let mut index = WorkspaceMemberIndex::default();
    let mut touched_raw: HashSet<SemanticId> = HashSet::new();
    let mut touched_canonical: HashSet<OwnerId> = HashSet::new();
    for file_id in db.vfs().file_ids() {
        let Some(cache) = db.file_cache(file_id) else {
            continue;
        };
        let exports = cache.exports.as_ref();
        if !file_matches_workspace_id(db, file_id, ws_id) {
            continue;
        }
        let (keys, canonical_keys) =
            index.add_file_inner(exports, &mut touched_raw, &mut touched_canonical);
        index.by_file.insert(file_id, keys);
        index.by_file_owner_id.insert(file_id, canonical_keys);
    }
    index.rebuild_touched_caches(touched_raw, touched_canonical);
    index
}

/// Merge one raw owner bucket across workspaces, returning the single workspace
/// bucket `Arc` when possible so repeated queries can clone it.
pub(crate) fn aggregate_member_bucket(
    ws_ids: &[WorkspaceId],
    members: &HashMap<WorkspaceId, WorkspaceMemberIndex>,
    owner: &SemanticId,
    named: Option<&str>,
) -> Option<Arc<[MemberRef]>> {
    let mut single: Option<Arc<[MemberRef]>> = None;
    let mut extra: Vec<MemberRef> = Vec::new();
    for &ws_id in ws_ids {
        let Some(index) = members.get(&ws_id) else {
            continue;
        };
        let bucket = match named {
            Some(name) => index.members_of_owner_named(owner, name),
            None => index.members_of_owner(owner),
        };
        let Some(bucket) = bucket else {
            continue;
        };
        if bucket.is_empty() {
            continue;
        }
        if single.is_none() && extra.is_empty() {
            single = Some(bucket);
        } else {
            if let Some(first) = single.take() {
                extra.extend(first.iter().cloned());
            }
            extra.extend(bucket.iter().cloned());
        }
    }
    match single {
        Some(bucket) => Some(bucket),
        None => (!extra.is_empty()).then(|| Arc::from(extra)),
    }
}

/// Build cross-workspace raw owner / named owner aggregate caches.
pub(crate) fn build_member_aggregates(
    ws_ids: &[WorkspaceId],
    members: &HashMap<WorkspaceId, WorkspaceMemberIndex>,
) -> (
    HashMap<SemanticId, Arc<[MemberRef]>>,
    HashMap<(SemanticId, SmolStr), Arc<[MemberRef]>>,
) {
    let mut owner_keys: HashSet<SemanticId> = HashSet::new();
    let mut name_keys: HashSet<(SemanticId, SmolStr)> = HashSet::new();
    for &ws_id in ws_ids {
        let Some(index) = members.get(&ws_id) else {
            continue;
        };
        owner_keys.extend(index.owner_ids());
        name_keys.extend(index.owner_name_keys());
    }

    let owner_members = owner_keys
        .into_iter()
        .filter_map(|owner| {
            aggregate_member_bucket(ws_ids, members, &owner, None).map(|bucket| (owner, bucket))
        })
        .collect();

    let owner_members_named = name_keys
        .into_iter()
        .filter_map(|key| {
            aggregate_member_bucket(ws_ids, members, &key.0, Some(key.1.as_str()))
                .map(|bucket| (key, bucket))
        })
        .collect();

    (owner_members, owner_members_named)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rowan::{TextRange, TextSize};

    fn member(file: u32, at: u32, name: &str) -> MemberRef {
        let file_id = FileId { id: file };
        let range = TextRange::at(TextSize::from(at), TextSize::from(1));
        MemberRef {
            file_id,
            id: SemanticId::member(file_id, range),
            name: SmolStr::new(name),
        }
    }

    fn offsets(members: &[MemberRef]) -> Vec<u32> {
        members
            .iter()
            .map(|member| match &member.id {
                SemanticId::Member(key) => u32::from(key.key_range.start()),
                _ => u32::MAX,
            })
            .collect()
    }

    fn frozen(members: impl IntoIterator<Item = MemberRef>) -> OwnerMembers {
        let mut bucket = OwnerMembers::default();
        for member in members {
            bucket.insert(member);
        }
        bucket.rebuild_caches();
        bucket
    }

    #[test]
    fn single_name_bucket_answers_named_from_all() {
        let bucket = frozen([member(1, 0, "x"), member(2, 1, "x")]);
        assert_eq!(bucket.names().count(), 1);
        assert_eq!(offsets(&bucket.named("x").expect("x")), vec![0, 1]);
        assert!(bucket.named("y").is_none());
    }

    #[test]
    fn multi_name_bucket_keeps_declaration_order_per_name() {
        let bucket = frozen([member(1, 0, "x"), member(1, 1, "y"), member(2, 2, "x")]);
        let mut names: Vec<_> = bucket.names().map(|name| name.to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["x", "y"]);
        assert_eq!(offsets(&bucket.named("x").expect("x")), vec![0, 2]);
        assert_eq!(offsets(&bucket.named("y").expect("y")), vec![1]);
        assert_eq!(offsets(&bucket.all()), vec![0, 1, 2]);
    }

    #[test]
    fn duplicate_insert_is_rejected_and_leaves_bucket_frozen() {
        let mut bucket = frozen([member(1, 0, "x")]);
        assert!(!bucket.insert(member(1, 0, "x")));
        assert!(bucket.open.is_none());
        assert!(bucket.insert(member(1, 5, "z")));
        bucket.rebuild_caches();
        assert_eq!(offsets(&bucket.all()), vec![0, 5]);
    }

    #[test]
    fn remove_updates_all_and_names_after_rebuild() {
        let mut bucket = frozen([member(1, 0, "x"), member(1, 1, "y")]);
        assert!(bucket.remove(&member(1, 0, "x").id));
        assert!(!bucket.remove(&member(1, 0, "x").id));
        bucket.rebuild_caches();
        assert_eq!(offsets(&bucket.all()), vec![1]);
        assert!(bucket.named("x").is_none());
        assert_eq!(offsets(&bucket.named("y").expect("y")), vec![1]);
        assert!(bucket.remove(&member(1, 1, "y").id));
        assert!(bucket.is_empty());
    }

    #[test]
    fn large_bucket_dedups_through_the_id_set() {
        let mut bucket = OwnerMembers::default();
        for at in 0..100 {
            assert!(bucket.insert(member(1, at, "x")));
        }
        for at in 0..100 {
            assert!(!bucket.insert(member(1, at, "x")));
        }
        bucket.rebuild_caches();
        assert_eq!(bucket.all().len(), 100);
        assert!(bucket.remove(&member(1, 50, "x").id));
        bucket.rebuild_caches();
        assert_eq!(bucket.all().len(), 99);
    }
}
