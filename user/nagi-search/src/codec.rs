use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};
use core::str;

use nagi_model::{AppId, AppSessionId, ObjectId, WorkspaceId};

use crate::{
    model::{
        MetadataRecord, ObjectKind, Relation, RelationKind, RelationProvenance, VisibilityScope,
        Workspace, WorkspaceSession,
    },
    store::{
        MetadataStoreError, StoreState, CURRENT_STORE_VERSION, MAX_OBJECT_RECORDS, MAX_RELATIONS,
        MAX_SNAPSHOT_BYTES, MAX_WORKSPACES,
    },
};

const MAGIC: &[u8; 8] = b"NGIMETA\0";
const HEADER_BYTES: usize = 22;

pub(crate) fn encode(state: &StoreState) -> Result<Vec<u8>, MetadataStoreError> {
    if state.records.len() > MAX_OBJECT_RECORDS
        || state.relations.len() > MAX_RELATIONS
        || state.workspaces.len() > MAX_WORKSPACES
    {
        return Err(MetadataStoreError::Capacity);
    }
    let mut payload = Vec::new();
    write_u32(&mut payload, state.records.len())?;
    for record in state.records.values() {
        write_record(&mut payload, record)?;
    }
    write_u32(&mut payload, state.relations.len())?;
    for relation in &state.relations {
        write_u64(&mut payload, relation.source.0);
        payload.push(relation.kind as u8);
        write_u64(&mut payload, relation.target.0);
        payload.push(relation.provenance as u8);
    }
    write_u32(&mut payload, state.workspaces.len())?;
    for workspace in state.workspaces.values() {
        write_workspace(&mut payload, workspace)?;
    }

    if payload.len().saturating_add(HEADER_BYTES) > MAX_SNAPSHOT_BYTES {
        return Err(MetadataStoreError::Capacity);
    }
    let mut output = Vec::with_capacity(HEADER_BYTES + payload.len());
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&CURRENT_STORE_VERSION.to_le_bytes());
    output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    output.extend_from_slice(&checksum(&payload).to_le_bytes());
    output.extend_from_slice(&payload);
    Ok(output)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<StoreState, MetadataStoreError> {
    if bytes.len() < HEADER_BYTES || bytes.len() > MAX_SNAPSHOT_BYTES || &bytes[..8] != MAGIC {
        return Err(MetadataStoreError::CorruptSnapshot);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != CURRENT_STORE_VERSION {
        return Err(MetadataStoreError::UnsupportedVersion(version));
    }
    let payload_len = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    let expected_checksum = u64::from_le_bytes([
        bytes[14], bytes[15], bytes[16], bytes[17], bytes[18], bytes[19], bytes[20], bytes[21],
    ]);
    if payload_len != bytes.len() - HEADER_BYTES {
        return Err(MetadataStoreError::CorruptSnapshot);
    }
    let payload = &bytes[HEADER_BYTES..];
    if checksum(payload) != expected_checksum {
        return Err(MetadataStoreError::CorruptSnapshot);
    }

    let mut reader = Reader::new(payload);
    let record_count = reader.count(MAX_OBJECT_RECORDS)?;
    let mut records = BTreeMap::new();
    for _ in 0..record_count {
        let record = read_record(&mut reader)?;
        record
            .validate()
            .map_err(MetadataStoreError::InvalidRecord)?;
        if records.insert(record.object_id, record).is_some() {
            return Err(MetadataStoreError::CorruptSnapshot);
        }
    }
    let relation_count = reader.count(MAX_RELATIONS)?;
    let mut relations = BTreeSet::new();
    for _ in 0..relation_count {
        let source = ObjectId(reader.u64()?);
        let kind =
            RelationKind::from_code(reader.u8()?).ok_or(MetadataStoreError::CorruptSnapshot)?;
        let target = ObjectId(reader.u64()?);
        let provenance = RelationProvenance::from_code(reader.u8()?)
            .ok_or(MetadataStoreError::CorruptSnapshot)?;
        if !relations.insert(Relation {
            source,
            kind,
            target,
            provenance,
        }) {
            return Err(MetadataStoreError::CorruptSnapshot);
        }
    }
    let workspace_count = reader.count(MAX_WORKSPACES)?;
    let mut workspaces = BTreeMap::new();
    for _ in 0..workspace_count {
        let workspace = read_workspace(&mut reader)?;
        workspace
            .validate()
            .map_err(MetadataStoreError::InvalidRecord)?;
        if workspaces
            .insert(workspace.workspace_id, workspace)
            .is_some()
        {
            return Err(MetadataStoreError::CorruptSnapshot);
        }
    }
    if !reader.is_finished() {
        return Err(MetadataStoreError::CorruptSnapshot);
    }

    for relation in &relations {
        if !is_active(&records, relation.source) || !is_active(&records, relation.target) {
            return Err(MetadataStoreError::CorruptSnapshot);
        }
    }
    for workspace in workspaces.values() {
        if workspace.objects.windows(2).any(|ids| ids[0] >= ids[1])
            || workspace.sessions.windows(2).any(|ids| ids[0] >= ids[1])
            || workspace.objects.iter().any(|id| !is_active(&records, *id))
        {
            return Err(MetadataStoreError::CorruptSnapshot);
        }
    }
    Ok(StoreState {
        records,
        relations,
        workspaces,
    })
}

fn write_record(out: &mut Vec<u8>, record: &MetadataRecord) -> Result<(), MetadataStoreError> {
    write_u64(out, record.object_id.0);
    out.push(record.kind as u8);
    write_string(out, &record.title)?;
    write_option_string(out, record.location.as_deref())?;
    write_option_u64(out, record.source_app.map(|value| value.0));
    write_option_u64(out, record.source_session.map(|value| value.0));
    write_option_i64(out, record.created_at);
    write_option_i64(out, record.modified_at);
    write_option_i64(out, record.observed_at);
    write_u16(out, record.tags.len())?;
    for tag in &record.tags {
        write_string(out, tag)?;
    }
    write_attributes(out, &record.attributes)?;
    out.push(record.visibility as u8);
    write_option_i64(out, record.tombstoned_at);
    Ok(())
}

fn read_record(reader: &mut Reader<'_>) -> Result<MetadataRecord, MetadataStoreError> {
    let object_id = ObjectId(reader.u64()?);
    let kind = ObjectKind::from_code(reader.u8()?).ok_or(MetadataStoreError::CorruptSnapshot)?;
    let title = reader.string(crate::model::MAX_TITLE_BYTES)?;
    let location = reader.option_string(crate::model::MAX_LOCATION_BYTES)?;
    let source_app = reader.option_u64()?.map(AppId);
    let source_session = reader.option_u64()?.map(AppSessionId);
    let created_at = reader.option_i64()?;
    let modified_at = reader.option_i64()?;
    let observed_at = reader.option_i64()?;
    let tag_count = reader.count16(crate::model::MAX_TAGS)?;
    let mut tags = Vec::with_capacity(tag_count);
    for _ in 0..tag_count {
        tags.push(reader.string(crate::model::MAX_TAG_BYTES)?);
    }
    let attributes = read_attributes(reader)?;
    let visibility =
        VisibilityScope::from_code(reader.u8()?).ok_or(MetadataStoreError::CorruptSnapshot)?;
    let tombstoned_at = reader.option_i64()?;
    Ok(MetadataRecord {
        object_id,
        kind,
        title,
        location,
        source_app,
        source_session,
        created_at,
        modified_at,
        observed_at,
        tags,
        attributes,
        visibility,
        tombstoned_at,
    })
}

fn write_workspace(out: &mut Vec<u8>, workspace: &Workspace) -> Result<(), MetadataStoreError> {
    write_u64(out, workspace.workspace_id.0);
    write_string(out, &workspace.title)?;
    write_option_u64(out, workspace.owner_app.map(|value| value.0));
    out.push(workspace.visibility as u8);
    write_option_i64(out, workspace.created_at);
    write_option_i64(out, workspace.modified_at);
    write_u16(out, workspace.tags.len())?;
    for tag in &workspace.tags {
        write_string(out, tag)?;
    }
    write_attributes(out, &workspace.attributes)?;
    write_u16(out, workspace.sessions.len())?;
    for session in &workspace.sessions {
        write_u64(out, session.app_id.0);
        write_u64(out, session.session_id.0);
    }
    write_u32(out, workspace.objects.len())?;
    for object in &workspace.objects {
        write_u64(out, object.0);
    }
    Ok(())
}

fn read_workspace(reader: &mut Reader<'_>) -> Result<Workspace, MetadataStoreError> {
    let workspace_id = WorkspaceId(reader.u64()?);
    let title = reader.string(crate::model::MAX_TITLE_BYTES)?;
    let owner_app = reader.option_u64()?.map(AppId);
    let visibility =
        VisibilityScope::from_code(reader.u8()?).ok_or(MetadataStoreError::CorruptSnapshot)?;
    let created_at = reader.option_i64()?;
    let modified_at = reader.option_i64()?;
    let tag_count = reader.count16(crate::model::MAX_TAGS)?;
    let mut tags = Vec::with_capacity(tag_count);
    for _ in 0..tag_count {
        tags.push(reader.string(crate::model::MAX_TAG_BYTES)?);
    }
    let attributes = read_attributes(reader)?;
    let session_count = reader.count16(crate::model::MAX_WORKSPACE_SESSIONS)?;
    let mut sessions = Vec::with_capacity(session_count);
    for _ in 0..session_count {
        sessions.push(WorkspaceSession {
            app_id: AppId(reader.u64()?),
            session_id: AppSessionId(reader.u64()?),
        });
    }
    let object_count = reader.count(crate::model::MAX_WORKSPACE_OBJECTS)?;
    let mut objects = Vec::with_capacity(object_count);
    for _ in 0..object_count {
        objects.push(ObjectId(reader.u64()?));
    }
    Ok(Workspace {
        workspace_id,
        title,
        owner_app,
        visibility,
        created_at,
        modified_at,
        tags,
        attributes,
        sessions,
        objects,
    })
}

fn write_attributes(
    out: &mut Vec<u8>,
    attributes: &BTreeMap<String, String>,
) -> Result<(), MetadataStoreError> {
    write_u16(out, attributes.len())?;
    for (key, value) in attributes {
        write_string(out, key)?;
        write_string(out, value)?;
    }
    Ok(())
}

fn read_attributes(
    reader: &mut Reader<'_>,
) -> Result<BTreeMap<String, String>, MetadataStoreError> {
    let count = reader.count16(crate::model::MAX_ATTRIBUTES)?;
    let mut attributes = BTreeMap::new();
    for _ in 0..count {
        let key = reader.string(crate::model::MAX_ATTRIBUTE_BYTES)?;
        let value = reader.string(crate::model::MAX_ATTRIBUTE_BYTES)?;
        if attributes.insert(key, value).is_some() {
            return Err(MetadataStoreError::CorruptSnapshot);
        }
    }
    Ok(attributes)
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), MetadataStoreError> {
    write_u32(out, value.len())?;
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn write_option_string(out: &mut Vec<u8>, value: Option<&str>) -> Result<(), MetadataStoreError> {
    match value {
        Some(value) => {
            out.push(1);
            write_string(out, value)?;
        }
        None => out.push(0),
    }
    Ok(())
}

fn write_option_u64(out: &mut Vec<u8>, value: Option<u64>) {
    match value {
        Some(value) => {
            out.push(1);
            write_u64(out, value);
        }
        None => out.push(0),
    }
}

fn write_option_i64(out: &mut Vec<u8>, value: Option<i64>) {
    match value {
        Some(value) => {
            out.push(1);
            out.extend_from_slice(&value.to_le_bytes());
        }
        None => out.push(0),
    }
}

fn write_u16(out: &mut Vec<u8>, value: usize) -> Result<(), MetadataStoreError> {
    let value = u16::try_from(value).map_err(|_| MetadataStoreError::Capacity)?;
    out.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_u32(out: &mut Vec<u8>, value: usize) -> Result<(), MetadataStoreError> {
    let value = u32::try_from(value).map_err(|_| MetadataStoreError::Capacity)?;
    out.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x100_0000_01b3);
    }
    value
}

fn is_active(records: &BTreeMap<ObjectId, MetadataRecord>, id: ObjectId) -> bool {
    records
        .get(&id)
        .is_some_and(|record| record.tombstoned_at.is_none())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_finished(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], MetadataStoreError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(MetadataStoreError::CorruptSnapshot)?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(MetadataStoreError::CorruptSnapshot)?;
        self.offset = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, MetadataStoreError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, MetadataStoreError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, MetadataStoreError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, MetadataStoreError> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn i64(&mut self) -> Result<i64, MetadataStoreError> {
        let bytes = self.take(8)?;
        Ok(i64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn count(&mut self, maximum: usize) -> Result<usize, MetadataStoreError> {
        let count = self.u32()? as usize;
        if count > maximum {
            return Err(MetadataStoreError::CorruptSnapshot);
        }
        Ok(count)
    }

    fn count16(&mut self, maximum: usize) -> Result<usize, MetadataStoreError> {
        let count = self.u16()? as usize;
        if count > maximum {
            return Err(MetadataStoreError::CorruptSnapshot);
        }
        Ok(count)
    }

    fn string(&mut self, maximum: usize) -> Result<String, MetadataStoreError> {
        let length = self.u32()? as usize;
        if length > maximum {
            return Err(MetadataStoreError::CorruptSnapshot);
        }
        let bytes = self.take(length)?;
        let value = str::from_utf8(bytes).map_err(|_| MetadataStoreError::CorruptSnapshot)?;
        Ok(String::from(value))
    }

    fn option_string(&mut self, maximum: usize) -> Result<Option<String>, MetadataStoreError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.string(maximum)?)),
            _ => Err(MetadataStoreError::CorruptSnapshot),
        }
    }

    fn option_u64(&mut self) -> Result<Option<u64>, MetadataStoreError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u64()?)),
            _ => Err(MetadataStoreError::CorruptSnapshot),
        }
    }

    fn option_i64(&mut self) -> Result<Option<i64>, MetadataStoreError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.i64()?)),
            _ => Err(MetadataStoreError::CorruptSnapshot),
        }
    }
}
