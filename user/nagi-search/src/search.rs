use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::cmp::Ordering;

use nagi_model::{ObjectId, WorkspaceId};

use crate::{
    model::{
        AccessContext, AttributeMatch, MetadataRecord, ModelError, Relation, RelationDirection,
        SearchError, SearchHit, SearchMatch, SearchQuery, SearchResponse, SearchSort,
        VisibilityFilter, Workspace, WorkspaceGroup, WorkspaceHit,
    },
    store::{MetadataStore, MetadataStoreError, SnapshotBackend},
};

const MAX_RELATION_DEPTH: usize = 8;
const MAX_RELATION_NODES: usize = 256;

#[derive(Clone, Copy, Debug, Default)]
pub struct DenyAllVisibility;

impl VisibilityFilter for DenyAllVisibility {
    fn can_read_object(&self, _access: AccessContext, _record: &MetadataRecord) -> bool {
        false
    }

    fn can_read_workspace(&self, _access: AccessContext, _workspace: &Workspace) -> bool {
        false
    }
}

/// Search always receives an explicit policy implementation. `AccessContext`
/// is input to that policy, not proof of authority; production integration
/// must derive the context from a trusted capability-checked caller.
pub struct SearchService<B, V> {
    store: MetadataStore<B>,
    visibility: V,
}

impl<B: SnapshotBackend, V: VisibilityFilter> SearchService<B, V> {
    pub fn open(backend: B, visibility: V) -> Result<Self, MetadataStoreError> {
        Ok(Self {
            store: MetadataStore::open(backend)?,
            visibility,
        })
    }

    pub fn upsert_record(&mut self, record: MetadataRecord) -> Result<(), MetadataStoreError> {
        self.store.upsert_record(record)
    }

    pub fn remove_record(
        &mut self,
        object_id: ObjectId,
        deleted_at: i64,
    ) -> Result<bool, MetadataStoreError> {
        self.store.remove_record(object_id, deleted_at)
    }

    pub fn upsert_workspace(&mut self, workspace: Workspace) -> Result<(), MetadataStoreError> {
        self.store.upsert_workspace(workspace)
    }

    pub fn remove_workspace(
        &mut self,
        workspace_id: WorkspaceId,
    ) -> Result<bool, MetadataStoreError> {
        self.store.remove_workspace(workspace_id)
    }

    pub fn add_workspace_object(
        &mut self,
        workspace_id: WorkspaceId,
        object_id: ObjectId,
    ) -> Result<bool, MetadataStoreError> {
        self.store.add_workspace_object(workspace_id, object_id)
    }

    pub fn remove_workspace_object(
        &mut self,
        workspace_id: WorkspaceId,
        object_id: ObjectId,
    ) -> Result<bool, MetadataStoreError> {
        self.store.remove_workspace_object(workspace_id, object_id)
    }

    pub fn link_session(
        &mut self,
        workspace_id: WorkspaceId,
        session: crate::WorkspaceSession,
    ) -> Result<bool, MetadataStoreError> {
        self.store.link_session(workspace_id, session)
    }

    pub fn add_relation(&mut self, relation: Relation) -> Result<bool, MetadataStoreError> {
        self.store.add_relation(relation)
    }

    pub fn remove_relation(&mut self, relation: Relation) -> Result<bool, MetadataStoreError> {
        self.store.remove_relation(relation)
    }

    /// An unauthorized or unknown identity has the same observable result:
    /// `None`. Callers cannot use this API to probe metadata existence.
    pub fn get_object(&self, access: AccessContext, object_id: ObjectId) -> Option<MetadataRecord> {
        let record = self.store.record(object_id)?;
        if record.tombstoned_at.is_some() || !self.visibility.can_read_object(access, record) {
            return None;
        }
        Some(record.clone())
    }

    pub fn get_workspace(
        &self,
        access: AccessContext,
        workspace_id: WorkspaceId,
    ) -> Option<Workspace> {
        let workspace = self.store.workspace(workspace_id)?;
        if !self.visibility.can_read_workspace(access, workspace) {
            return None;
        }
        let mut visible = workspace.clone();
        visible
            .objects
            .retain(|id| self.visible_object(access, *id).is_some());
        visible.sessions.retain(|session| {
            self.visibility
                .can_read_workspace_session(access, workspace, *session)
        });
        Some(visible)
    }

    /// Returns only caller-visible matches. There is intentionally no total
    /// database count, facet count, or pre-filter result count in this API.
    pub fn search(
        &self,
        access: AccessContext,
        query: &SearchQuery,
    ) -> Result<SearchResponse, SearchError> {
        validate_query(query)?;
        if query.limit == 0 {
            return Ok(SearchResponse::default());
        }

        let scoped_workspace = match query.workspace {
            Some(id) => match self.visible_workspace(access, id) {
                Some(workspace) => Some(workspace),
                None => return Ok(SearchResponse::default()),
            },
            None => None,
        };
        let related_record = match query.related_to {
            Some(id) => match self.visible_object(access, id) {
                Some(record) => Some(record),
                None => return Ok(SearchResponse::default()),
            },
            None => None,
        };
        let relation_map = related_record
            .as_ref()
            .map(|related| self.visible_relation_matches(access, related.object_id, query));

        let mut objects = Vec::new();
        for record in self.store.records() {
            // This check precedes any query field inspection or rationale
            // construction so hidden metadata cannot affect observable output.
            if record.tombstoned_at.is_some() || !self.visibility.can_read_object(access, record) {
                continue;
            }
            if scoped_workspace
                .is_some_and(|workspace| !workspace.objects.contains(&record.object_id))
            {
                continue;
            }
            let relation_matches = relation_map
                .as_ref()
                .and_then(|matches| matches.get(&record.object_id))
                .cloned()
                .unwrap_or_default();
            if related_record.is_some() && relation_matches.is_empty() {
                continue;
            }
            if let Some(hit) = match_object(record, query, relation_matches) {
                objects.push(hit);
            }
        }
        sort_hits(&mut objects, query.sort);
        objects.truncate(query.limit);

        let mut workspaces = Vec::new();
        if query.kind.is_none() && query.related_to.is_none() {
            for workspace in self.store.workspaces() {
                // Workspace authorization also precedes field inspection.
                if !self.visibility.can_read_workspace(access, workspace) {
                    continue;
                }
                if let Some(hit) = match_workspace(workspace, query) {
                    workspaces.push(hit);
                }
            }
            sort_workspace_hits(&mut workspaces, query.sort);
            workspaces.truncate(query.limit);
        }

        let visible_ids: BTreeSet<ObjectId> =
            objects.iter().map(|hit| hit.record.object_id).collect();
        let result_order = objects
            .iter()
            .enumerate()
            .map(|(index, hit)| (hit.record.object_id, index))
            .collect::<alloc::collections::BTreeMap<_, _>>();
        let mut groups = Vec::new();
        for workspace in self.store.workspaces() {
            if query
                .workspace
                .is_some_and(|id| id != workspace.workspace_id)
            {
                continue;
            }
            if !self.visibility.can_read_workspace(access, workspace) {
                continue;
            }
            let mut object_ids: Vec<ObjectId> = workspace
                .objects
                .iter()
                .copied()
                .filter(|id| visible_ids.contains(id))
                .collect();
            if !object_ids.is_empty() {
                object_ids.sort_by_key(|id| result_order.get(id).copied().unwrap_or(usize::MAX));
                groups.push(WorkspaceGroup {
                    workspace_id: workspace.workspace_id,
                    title: workspace.title.clone(),
                    object_ids,
                });
            }
        }
        groups.sort_by_key(|group| group.workspace_id);

        Ok(SearchResponse {
            objects,
            workspaces,
            workspace_groups: groups,
        })
    }

    /// Traverse only direct stored relations, in stable ObjectId order. A
    /// denied node is not returned and is not used as a bridge to other nodes.
    pub fn related_objects(
        &self,
        access: AccessContext,
        start: ObjectId,
        direction: RelationDirection,
        max_depth: usize,
        max_nodes: usize,
    ) -> Result<Vec<ObjectId>, SearchError> {
        if max_depth == 0
            || max_depth > MAX_RELATION_DEPTH
            || max_nodes == 0
            || max_nodes > MAX_RELATION_NODES
        {
            return Err(SearchError::InvalidTraversalBound);
        }
        if self.visible_object(access, start).is_none() {
            return Ok(Vec::new());
        }

        let mut visited = BTreeSet::new();
        visited.insert(start);
        let mut frontier = vec![start];
        let mut results = Vec::new();
        for _ in 0..max_depth {
            let mut next_frontier = BTreeSet::new();
            for current in frontier {
                for relation in self.store.relations() {
                    let candidate = match direction {
                        RelationDirection::Either if relation.source == current => {
                            Some(relation.target)
                        }
                        RelationDirection::Either if relation.target == current => {
                            Some(relation.source)
                        }
                        RelationDirection::Outgoing if relation.source == current => {
                            Some(relation.target)
                        }
                        RelationDirection::Incoming if relation.target == current => {
                            Some(relation.source)
                        }
                        _ => None,
                    };
                    let Some(candidate) = candidate else { continue };
                    if visited.contains(&candidate)
                        || self.visible_object(access, candidate).is_none()
                    {
                        continue;
                    }
                    if results.len() >= max_nodes {
                        break;
                    }
                    visited.insert(candidate);
                    next_frontier.insert(candidate);
                    results.push(candidate);
                }
            }
            if next_frontier.is_empty() || results.len() >= max_nodes {
                break;
            }
            frontier = next_frontier.into_iter().collect();
        }
        Ok(results)
    }

    fn visible_object(&self, access: AccessContext, id: ObjectId) -> Option<&MetadataRecord> {
        let record = self.store.record(id)?;
        (record.tombstoned_at.is_none() && self.visibility.can_read_object(access, record))
            .then_some(record)
    }

    fn visible_workspace(&self, access: AccessContext, id: WorkspaceId) -> Option<&Workspace> {
        let workspace = self.store.workspace(id)?;
        self.visibility
            .can_read_workspace(access, workspace)
            .then_some(workspace)
    }

    fn visible_relation_matches(
        &self,
        access: AccessContext,
        related: ObjectId,
        query: &SearchQuery,
    ) -> alloc::collections::BTreeMap<ObjectId, Vec<SearchMatch>> {
        let mut matches = alloc::collections::BTreeMap::new();
        for relation in self.store.relations() {
            let candidate = match query.relation_direction {
                RelationDirection::Either if relation.source == related => Some(relation.target),
                RelationDirection::Either if relation.target == related => Some(relation.source),
                RelationDirection::Outgoing if relation.source == related => Some(relation.target),
                RelationDirection::Incoming if relation.target == related => Some(relation.source),
                _ => None,
            };
            let Some(candidate) = candidate else { continue };
            // Both endpoints are authorized before the relation rationale
            // reaches the caller; provenance remains explicit.
            if self.visible_object_for_relation_endpoint(access, candidate)
                && self.visible_object_for_relation_endpoint(access, related)
            {
                matches
                    .entry(candidate)
                    .or_insert_with(Vec::new)
                    .push(SearchMatch::Relation {
                        kind: relation.kind,
                        provenance: relation.provenance,
                    });
            }
        }
        matches
    }

    fn visible_object_for_relation_endpoint(&self, access: AccessContext, id: ObjectId) -> bool {
        let Some(record) = self.store.record(id) else {
            return false;
        };
        record.tombstoned_at.is_none() && self.visibility.can_read_object(access, record)
    }
}

fn validate_query(query: &SearchQuery) -> Result<(), SearchError> {
    if query.limit > crate::model::MAX_SEARCH_RESULTS {
        return Err(SearchError::InvalidModel(ModelError::SearchLimitExceeded));
    }
    if query
        .text
        .as_ref()
        .is_some_and(|text| text.trim().is_empty())
    {
        return Err(SearchError::InvalidModel(ModelError::EmptyTextQuery));
    }
    for tag in &query.tags_any {
        if tag.trim().is_empty() {
            return Err(SearchError::InvalidModel(ModelError::EmptyTag));
        }
    }
    for AttributeMatch { key, .. } in &query.attributes_all {
        if key.trim().is_empty() {
            return Err(SearchError::InvalidModel(ModelError::EmptyAttributeKey));
        }
    }
    for range in [query.created, query.modified, query.observed]
        .into_iter()
        .flatten()
    {
        range.validate().map_err(SearchError::InvalidModel)?;
    }
    Ok(())
}

fn match_object(
    record: &MetadataRecord,
    query: &SearchQuery,
    relation_matches: Vec<SearchMatch>,
) -> Option<SearchHit> {
    if query.kind.is_some_and(|kind| kind != record.kind)
        || query
            .source_app
            .is_some_and(|app| record.source_app != Some(app))
    {
        return None;
    }
    let mut rationale = Vec::new();
    if let Some(text) = &query.text {
        let text = text.to_lowercase();
        let title = record.title.to_lowercase();
        let mut text_matched = false;
        if title == text {
            rationale.push(SearchMatch::TitleExact);
            text_matched = true;
        } else if title.starts_with(&text) {
            rationale.push(SearchMatch::TitlePrefix);
            text_matched = true;
        } else if title.contains(&text) {
            rationale.push(SearchMatch::TitleSubstring);
            text_matched = true;
        }
        for tag in &record.tags {
            if tag.to_lowercase().contains(&text) {
                rationale.push(SearchMatch::Tag(tag.clone()));
                text_matched = true;
            }
        }
        for (key, value) in &record.attributes {
            if key.to_lowercase().contains(&text) || value.to_lowercase().contains(&text) {
                rationale.push(SearchMatch::Attribute(key.clone()));
                text_matched = true;
            }
        }
        if !text_matched {
            return None;
        }
    }
    if query.kind.is_some() {
        rationale.push(SearchMatch::Kind);
    }
    if query.source_app.is_some() {
        rationale.push(SearchMatch::SourceApplication);
    }
    if !query.tags_any.is_empty() {
        let matched: Vec<String> = query
            .tags_any
            .iter()
            .filter(|needle| {
                record
                    .tags
                    .iter()
                    .any(|tag| tag.eq_ignore_ascii_case(needle))
            })
            .map(ToString::to_string)
            .collect();
        if matched.is_empty() {
            return None;
        }
        rationale.extend(matched.into_iter().map(SearchMatch::Tag));
    }
    for attribute in &query.attributes_all {
        if !record.attributes.iter().any(|(key, value)| {
            key.eq_ignore_ascii_case(&attribute.key) && value.eq_ignore_ascii_case(&attribute.value)
        }) {
            return None;
        }
        rationale.push(SearchMatch::Attribute(attribute.key.clone()));
    }
    for (range, value, reason) in [
        (query.created, record.created_at, SearchMatch::CreatedTime),
        (
            query.modified,
            record.modified_at,
            SearchMatch::ModifiedTime,
        ),
        (
            query.observed,
            record.observed_at,
            SearchMatch::ObservedTime,
        ),
    ] {
        if let Some(range) = range {
            if !range.contains(value) {
                return None;
            }
            rationale.push(reason);
        }
    }
    if let Some(workspace_id) = query.workspace {
        rationale.push(SearchMatch::Workspace(workspace_id));
    }
    rationale.extend(relation_matches);
    Some(SearchHit {
        record: record.clone(),
        rationale,
    })
}

fn match_workspace(workspace: &Workspace, query: &SearchQuery) -> Option<WorkspaceHit> {
    if query
        .workspace
        .is_some_and(|id| id != workspace.workspace_id)
    {
        return None;
    }
    if query
        .source_app
        .is_some_and(|app| workspace.owner_app != Some(app))
    {
        return None;
    }
    let mut rationale = Vec::new();
    if let Some(text) = &query.text {
        let needle = text.to_lowercase();
        let title = workspace.title.to_lowercase();
        if title == needle {
            rationale.push(SearchMatch::TitleExact);
        } else if title.starts_with(&needle) {
            rationale.push(SearchMatch::TitlePrefix);
        } else if title.contains(&needle) {
            rationale.push(SearchMatch::TitleSubstring);
        } else {
            for tag in &workspace.tags {
                if tag.to_lowercase().contains(&needle) {
                    rationale.push(SearchMatch::Tag(tag.clone()));
                }
            }
            for (key, value) in &workspace.attributes {
                if key.to_lowercase().contains(&needle) || value.to_lowercase().contains(&needle) {
                    rationale.push(SearchMatch::Attribute(key.clone()));
                }
            }
            if rationale.is_empty() {
                return None;
            }
        }
    }
    if query.source_app.is_some() {
        rationale.push(SearchMatch::SourceApplication);
    }
    if !query.tags_any.is_empty() {
        let matched: Vec<String> = query
            .tags_any
            .iter()
            .filter(|needle| {
                workspace
                    .tags
                    .iter()
                    .any(|tag| tag.eq_ignore_ascii_case(needle))
            })
            .map(ToString::to_string)
            .collect();
        if matched.is_empty() {
            return None;
        }
        rationale.extend(matched.into_iter().map(SearchMatch::Tag));
    }
    for attribute in &query.attributes_all {
        if !workspace.attributes.iter().any(|(key, value)| {
            key.eq_ignore_ascii_case(&attribute.key) && value.eq_ignore_ascii_case(&attribute.value)
        }) {
            return None;
        }
        rationale.push(SearchMatch::Attribute(attribute.key.clone()));
    }
    if let Some(range) = query.created {
        if !range.contains(workspace.created_at) {
            return None;
        }
        rationale.push(SearchMatch::CreatedTime);
    }
    if let Some(range) = query.modified {
        if !range.contains(workspace.modified_at) {
            return None;
        }
        rationale.push(SearchMatch::ModifiedTime);
    }
    Some(WorkspaceHit {
        workspace_id: workspace.workspace_id,
        title: workspace.title.clone(),
        modified_at: workspace.modified_at,
        rationale,
    })
}

fn sort_hits(hits: &mut [SearchHit], sort: SearchSort) {
    hits.sort_by(|left, right| match sort {
        SearchSort::Relevance => relevance(right)
            .cmp(&relevance(left))
            .then_with(|| right.record.modified_at.cmp(&left.record.modified_at))
            .then_with(|| left.record.object_id.cmp(&right.record.object_id)),
        SearchSort::ModifiedNewest => right
            .record
            .modified_at
            .cmp(&left.record.modified_at)
            .then_with(|| left.record.object_id.cmp(&right.record.object_id)),
        SearchSort::TitleAscending => left
            .record
            .title
            .to_lowercase()
            .cmp(&right.record.title.to_lowercase())
            .then_with(|| left.record.object_id.cmp(&right.record.object_id)),
        SearchSort::ObjectIdAscending => left.record.object_id.cmp(&right.record.object_id),
    });
}

fn relevance(hit: &SearchHit) -> u8 {
    if hit.rationale.contains(&SearchMatch::TitleExact) {
        4
    } else if hit.rationale.contains(&SearchMatch::TitlePrefix) {
        3
    } else if hit.rationale.contains(&SearchMatch::TitleSubstring) {
        2
    } else {
        1
    }
}

fn sort_workspace_hits(hits: &mut [WorkspaceHit], sort: SearchSort) {
    hits.sort_by(|left, right| {
        let ordering = match sort {
            SearchSort::Relevance => workspace_relevance(right)
                .cmp(&workspace_relevance(left))
                .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase())),
            SearchSort::ModifiedNewest => right
                .modified_at
                .cmp(&left.modified_at)
                .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase())),
            SearchSort::TitleAscending => {
                left.title.to_lowercase().cmp(&right.title.to_lowercase())
            }
            SearchSort::ObjectIdAscending => Ordering::Equal,
        };
        ordering.then_with(|| left.workspace_id.cmp(&right.workspace_id))
    });
}

fn workspace_relevance(hit: &WorkspaceHit) -> u8 {
    if hit.rationale.contains(&SearchMatch::TitleExact) {
        4
    } else if hit.rationale.contains(&SearchMatch::TitlePrefix) {
        3
    } else if hit.rationale.contains(&SearchMatch::TitleSubstring) {
        2
    } else {
        1
    }
}
