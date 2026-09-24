use crate::{
    Actor, CapabilityId, CapabilityScope, DelegationGrant, DelegationId, DeviceClassId, DomainName,
    PermissionDecision, PermissionGrant, SystemServiceId,
};
use nagi_model::{AppId, ObjectId, UserId};

const RECORD_VERSION: u8 = 1;
pub const MAX_ENCODED_GRANT_BYTES: usize = 384;
pub const MAX_ENCODED_DELEGATION_BYTES: usize = 384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    BufferTooSmall,
    Truncated,
    InvalidRecord,
    UnsupportedVersion,
}

pub fn encode_permission_grant(
    grant: &PermissionGrant,
    output: &mut [u8],
) -> Result<usize, CodecError> {
    let mut writer = Writer::new(output);
    writer.byte(RECORD_VERSION)?;
    write_actor(&mut writer, grant.actor)?;
    writer.name(grant.capability.as_bytes())?;
    write_scope(&mut writer, grant.scope)?;
    writer.byte(encode_decision(grant.decision))?;
    writer.byte(u8::from(grant.allow_background))?;
    Ok(writer.length())
}

pub fn decode_permission_grant(input: &[u8]) -> Result<PermissionGrant, CodecError> {
    let mut reader = Reader::new(input);
    check_version(&mut reader)?;
    let actor = read_actor(&mut reader)?;
    let capability = CapabilityId::parse(reader.name()?).map_err(|_| CodecError::InvalidRecord)?;
    let scope = read_scope(&mut reader)?;
    let decision = decode_decision(reader.byte()?)?;
    let allow_background = read_bool(&mut reader)?;
    reader.finish()?;
    Ok(PermissionGrant {
        actor,
        capability,
        scope,
        decision,
        allow_background,
    })
}

pub fn encode_delegation_grant(
    grant: &DelegationGrant,
    output: &mut [u8],
) -> Result<usize, CodecError> {
    let mut writer = Writer::new(output);
    writer.byte(RECORD_VERSION)?;
    writer.u64(grant.id.0)?;
    writer.u64(grant.user.0)?;
    write_actor(&mut writer, grant.agent)?;
    writer.name(grant.capability.as_bytes())?;
    write_scope(&mut writer, grant.scope)?;
    writer.u64(grant.expires_at)?;
    writer.byte(u8::from(grant.allow_background))?;
    Ok(writer.length())
}

pub fn decode_delegation_grant(input: &[u8]) -> Result<DelegationGrant, CodecError> {
    let mut reader = Reader::new(input);
    check_version(&mut reader)?;
    let id = DelegationId(reader.u64()?);
    let user = UserId(reader.u64()?);
    let agent = read_actor(&mut reader)?;
    let capability = CapabilityId::parse(reader.name()?).map_err(|_| CodecError::InvalidRecord)?;
    let scope = read_scope(&mut reader)?;
    let expires_at = reader.u64()?;
    let allow_background = read_bool(&mut reader)?;
    reader.finish()?;
    if !matches!(agent, Actor::AiAgent(_)) || expires_at == 0 {
        return Err(CodecError::InvalidRecord);
    }
    Ok(DelegationGrant {
        id,
        user,
        agent,
        capability,
        scope,
        expires_at,
        allow_background,
    })
}

fn check_version(reader: &mut Reader<'_>) -> Result<(), CodecError> {
    match reader.byte()? {
        RECORD_VERSION => Ok(()),
        _ => Err(CodecError::UnsupportedVersion),
    }
}

fn write_actor(writer: &mut Writer<'_>, actor: Actor) -> Result<(), CodecError> {
    let (tag, id) = match actor {
        Actor::User(UserId(id)) => (0, id),
        Actor::SystemService(SystemServiceId(id)) => (1, id),
        Actor::FirstPartyApp(AppId(id)) => (2, id),
        Actor::ThirdPartyApp(AppId(id)) => (3, id),
        Actor::AiAgent(AppId(id)) => (4, id),
        Actor::BackgroundAutomation(id) => (5, id),
    };
    writer.byte(tag)?;
    writer.u64(id)
}

fn read_actor(reader: &mut Reader<'_>) -> Result<Actor, CodecError> {
    let tag = reader.byte()?;
    let id = reader.u64()?;
    match tag {
        0 => Ok(Actor::User(UserId(id))),
        1 => Ok(Actor::SystemService(SystemServiceId(id))),
        2 => Ok(Actor::FirstPartyApp(AppId(id))),
        3 => Ok(Actor::ThirdPartyApp(AppId(id))),
        4 => Ok(Actor::AiAgent(AppId(id))),
        5 => Ok(Actor::BackgroundAutomation(id)),
        _ => Err(CodecError::InvalidRecord),
    }
}

fn write_scope(writer: &mut Writer<'_>, scope: CapabilityScope) -> Result<(), CodecError> {
    match scope {
        CapabilityScope::Any => writer.byte(0),
        CapabilityScope::Object(ObjectId(id)) => {
            writer.byte(1)?;
            writer.u64(id)
        }
        CapabilityScope::Directory(ObjectId(id)) => {
            writer.byte(2)?;
            writer.u64(id)
        }
        CapabilityScope::ObjectWithinDirectory {
            root_directory: ObjectId(root_directory),
            object: ObjectId(object),
        } => {
            writer.byte(7)?;
            writer.u64(root_directory)?;
            writer.u64(object)
        }
        CapabilityScope::Domain(domain) => {
            writer.byte(3)?;
            writer.name(domain.as_bytes())
        }
        CapabilityScope::Localhost => writer.byte(6),
        CapabilityScope::DeviceClass(class) => {
            writer.byte(4)?;
            writer.name(class.as_bytes())
        }
        CapabilityScope::Device { class, id } => {
            writer.byte(5)?;
            writer.name(class.as_bytes())?;
            writer.u64(id)
        }
    }
}

fn read_scope(reader: &mut Reader<'_>) -> Result<CapabilityScope, CodecError> {
    match reader.byte()? {
        0 => Ok(CapabilityScope::Any),
        1 => Ok(CapabilityScope::Object(ObjectId(reader.u64()?))),
        2 => Ok(CapabilityScope::Directory(ObjectId(reader.u64()?))),
        7 => Ok(CapabilityScope::ObjectWithinDirectory {
            root_directory: ObjectId(reader.u64()?),
            object: ObjectId(reader.u64()?),
        }),
        3 => DomainName::parse(reader.name()?)
            .map(CapabilityScope::Domain)
            .map_err(|_| CodecError::InvalidRecord),
        6 => Ok(CapabilityScope::Localhost),
        4 => DeviceClassId::parse(reader.name()?)
            .map(CapabilityScope::DeviceClass)
            .map_err(|_| CodecError::InvalidRecord),
        5 => {
            let class =
                DeviceClassId::parse(reader.name()?).map_err(|_| CodecError::InvalidRecord)?;
            let id = reader.u64()?;
            Ok(CapabilityScope::Device { class, id })
        }
        _ => Err(CodecError::InvalidRecord),
    }
}

const fn encode_decision(decision: PermissionDecision) -> u8 {
    match decision {
        PermissionDecision::Allow => 0,
        PermissionDecision::Deny => 1,
        PermissionDecision::Ask => 2,
    }
}

fn decode_decision(value: u8) -> Result<PermissionDecision, CodecError> {
    match value {
        0 => Ok(PermissionDecision::Allow),
        1 => Ok(PermissionDecision::Deny),
        2 => Ok(PermissionDecision::Ask),
        _ => Err(CodecError::InvalidRecord),
    }
}

fn read_bool(reader: &mut Reader<'_>) -> Result<bool, CodecError> {
    match reader.byte()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CodecError::InvalidRecord),
    }
}

struct Writer<'a> {
    output: &'a mut [u8],
    offset: usize,
}

impl<'a> Writer<'a> {
    const fn new(output: &'a mut [u8]) -> Self {
        Self { output, offset: 0 }
    }

    fn byte(&mut self, value: u8) -> Result<(), CodecError> {
        self.write(&[value])
    }

    fn u64(&mut self, value: u64) -> Result<(), CodecError> {
        self.write(&value.to_le_bytes())
    }

    fn name(&mut self, value: &[u8]) -> Result<(), CodecError> {
        let length = u8::try_from(value.len()).map_err(|_| CodecError::InvalidRecord)?;
        self.byte(length)?;
        self.write(value)
    }

    fn write(&mut self, value: &[u8]) -> Result<(), CodecError> {
        let end = self
            .offset
            .checked_add(value.len())
            .ok_or(CodecError::BufferTooSmall)?;
        let destination = self
            .output
            .get_mut(self.offset..end)
            .ok_or(CodecError::BufferTooSmall)?;
        destination.copy_from_slice(value);
        self.offset = end;
        Ok(())
    }

    const fn length(&self) -> usize {
        self.offset
    }
}

struct Reader<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    const fn new(input: &'a [u8]) -> Self {
        Self { input, offset: 0 }
    }

    fn byte(&mut self) -> Result<u8, CodecError> {
        let byte = *self.input.get(self.offset).ok_or(CodecError::Truncated)?;
        self.offset += 1;
        Ok(byte)
    }

    fn u64(&mut self) -> Result<u64, CodecError> {
        let end = self.offset.checked_add(8).ok_or(CodecError::Truncated)?;
        let bytes: [u8; 8] = self
            .input
            .get(self.offset..end)
            .ok_or(CodecError::Truncated)?
            .try_into()
            .map_err(|_| CodecError::Truncated)?;
        self.offset = end;
        Ok(u64::from_le_bytes(bytes))
    }

    fn name(&mut self) -> Result<&'a [u8], CodecError> {
        let length = usize::from(self.byte()?);
        let end = self
            .offset
            .checked_add(length)
            .ok_or(CodecError::Truncated)?;
        let name = self
            .input
            .get(self.offset..end)
            .ok_or(CodecError::Truncated)?;
        self.offset = end;
        Ok(name)
    }

    fn finish(self) -> Result<(), CodecError> {
        if self.offset == self.input.len() {
            Ok(())
        } else {
            Err(CodecError::InvalidRecord)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_delegation_grant, encode_delegation_grant, CodecError};
    use crate::{Actor, CapabilityId, CapabilityScope, DelegationGrant, DelegationId};
    use nagi_model::{AppId, UserId};

    #[test]
    fn delegation_record_round_trips_and_rejects_trailing_data() {
        let grant = DelegationGrant {
            id: DelegationId(4),
            user: UserId(8),
            agent: Actor::AiAgent(AppId(12)),
            capability: CapabilityId::parse(b"files.read").expect("capability"),
            scope: CapabilityScope::Any,
            expires_at: 99,
            allow_background: true,
        };
        let mut bytes = [0; super::MAX_ENCODED_DELEGATION_BYTES];
        let length = encode_delegation_grant(&grant, &mut bytes).expect("encode");
        assert_eq!(decode_delegation_grant(&bytes[..length]), Ok(grant));
        assert_eq!(
            decode_delegation_grant(&bytes[..length + 1]),
            Err(CodecError::InvalidRecord)
        );
    }

    #[test]
    fn localhost_scope_round_trips_without_becoming_a_domain_scope() {
        let grant = crate::PermissionGrant {
            actor: Actor::ThirdPartyApp(AppId(2)),
            capability: CapabilityId::parse(b"network.access").expect("capability"),
            scope: CapabilityScope::Localhost,
            decision: crate::PermissionDecision::Allow,
            allow_background: false,
        };
        let mut bytes = [0; super::MAX_ENCODED_GRANT_BYTES];
        let length = super::encode_permission_grant(&grant, &mut bytes).expect("encode");
        assert_eq!(super::decode_permission_grant(&bytes[..length]), Ok(grant));
    }

    #[test]
    fn directory_scoped_object_grant_round_trips() {
        let grant = crate::PermissionGrant {
            actor: Actor::ThirdPartyApp(AppId(2)),
            capability: CapabilityId::parse(b"files.write").expect("capability"),
            scope: CapabilityScope::ObjectWithinDirectory {
                root_directory: nagi_model::ObjectId(4),
                object: nagi_model::ObjectId(5),
            },
            decision: crate::PermissionDecision::Allow,
            allow_background: false,
        };
        let mut bytes = [0; super::MAX_ENCODED_GRANT_BYTES];
        let length = super::encode_permission_grant(&grant, &mut bytes).expect("encode");
        assert_eq!(super::decode_permission_grant(&bytes[..length]), Ok(grant));
    }
}
