//! Typed, bounded task-local Notes launch payload candidate. This SCM_RIGHTS
//! framing is not Task41's NIPC service protocol and is not connected to the
//! current production launcher. It grants no session, store, or process
//! authority; a production caller needs an authorized Task41/Task24 transport.

use nagi_model::ObjectId;
use nagi_notes::NoteId;
use std::io;
#[cfg(test)]
use std::io::Write;
use std::num::NonZeroU64;
use std::os::fd::AsRawFd;
#[cfg(test)]
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::{UnixDatagram, UnixStream};

#[cfg(test)]
const MAGIC: &[u8; 4] = b"NGB1";
#[cfg(test)]
const VERSION: u16 = 1;
#[cfg(test)]
pub const BOOTSTRAP_BYTES: usize = 40;
#[cfg(test)]
const ATTACHMENT_COUNT: usize = 2;

/// IDs identify an instance-owned surface; they do not prove that it has been
/// displayed or received keyboard focus.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SurfaceId(NonZeroU64);

impl SurfaceId {
    pub fn new(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(Self)
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Parent-side channel endpoints. The parent remains the only owner of the
/// physical evdev and framebuffer descriptors and forwards events/frames.
pub(crate) struct ParentNotesChannels {
    pub(crate) input: UnixDatagram,
    pub(crate) display: UnixStream,
}

/// Child-side descriptors for the proposed app-specific launch payload:
/// input in slot 0, display-frame channel in slot 1. The current Task41 NIPC
/// endpoint does not accept this raw descriptor frame.
pub(crate) struct ChildNotesChannels {
    pub(crate) input: UnixDatagram,
    pub(crate) display: UnixStream,
}

pub(crate) struct NotesBootstrap {
    pub(crate) request_id: NonZeroU64,
    pub(crate) note_id: NoteId,
    pub(crate) surface_id: SurfaceId,
    pub(crate) channels: ChildNotesChannels,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(test)]
pub enum BootstrapError {
    Io(io::ErrorKind),
    ShortFrame,
    BadMagic,
    UnsupportedVersion,
    BadLength,
    NonZeroReserved,
    InvalidRequest,
    InvalidNote,
    InvalidSurface,
    InvalidSlotCount,
    UnexpectedAncillary,
    TruncatedAncillary,
    WrongDescriptorCount,
    DescriptorFlags,
}

#[cfg(test)]
impl From<io::Error> for BootstrapError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

/// Create one duplex input/control channel and one duplex display handoff.
/// The child endpoints are consumed by `send_notes_bootstrap`; parent endpoints
/// stay with the one Shell/display owner.
pub(crate) fn channel_pair() -> io::Result<(ParentNotesChannels, ChildNotesChannels)> {
    let (parent_input, child_input) = UnixDatagram::pair()?;
    let (child_display, parent_display) = UnixStream::pair()?;
    set_cloexec(parent_input.as_raw_fd())?;
    set_cloexec(child_input.as_raw_fd())?;
    set_cloexec(child_display.as_raw_fd())?;
    set_cloexec(parent_display.as_raw_fd())?;
    Ok((
        ParentNotesChannels {
            input: parent_input,
            display: parent_display,
        },
        ChildNotesChannels {
            input: child_input,
            display: child_display,
        },
    ))
}

/// Send typed metadata and exactly two owned descriptors over a dedicated
/// app-specific launch payload channel. Do not pass Task41's NIPC service
/// stream here. A failed send does not make the launch successful.
#[cfg(test)]
pub(crate) fn send_notes_bootstrap(
    endpoint: &UnixStream,
    request_id: NonZeroU64,
    note_id: NoteId,
    surface_id: SurfaceId,
    channels: ChildNotesChannels,
) -> Result<(), BootstrapError> {
    if note_id.0 == 0 {
        return Err(BootstrapError::InvalidNote);
    }
    let frame = encode(request_id, note_id, surface_id);
    send_frame_with_fds(
        endpoint,
        &frame,
        [channels.input.as_raw_fd(), channels.display.as_raw_fd()],
    )?;
    drop(channels);
    Ok(())
}

/// Receive a fixed app launch frame and two SCM_RIGHTS descriptors. This
/// parser is not an authentication or authorization check.
#[cfg(test)]
pub(crate) fn receive_notes_bootstrap(
    endpoint: &UnixStream,
) -> Result<NotesBootstrap, BootstrapError> {
    let (frame, descriptors) = receive_frame_with_fds(endpoint)?;
    let (request_id, note_id, surface_id) = decode(&frame)?;
    if descriptors.len() != ATTACHMENT_COUNT {
        return Err(BootstrapError::WrongDescriptorCount);
    }
    let mut descriptors = descriptors.into_iter();
    let input: UnixDatagram = descriptors.next().expect("count checked").into();
    let display: UnixStream = descriptors.next().expect("count checked").into();
    Ok(NotesBootstrap {
        request_id,
        note_id,
        surface_id,
        channels: ChildNotesChannels { input, display },
    })
}

#[cfg(test)]
fn encode(request_id: NonZeroU64, note_id: NoteId, surface_id: SurfaceId) -> [u8; BOOTSTRAP_BYTES] {
    let mut frame = [0u8; BOOTSTRAP_BYTES];
    frame[0..4].copy_from_slice(MAGIC);
    frame[4..6].copy_from_slice(&VERSION.to_le_bytes());
    frame[6..8].copy_from_slice(&(BOOTSTRAP_BYTES as u16).to_le_bytes());
    frame[8..12].copy_from_slice(&0u32.to_le_bytes());
    frame[12..20].copy_from_slice(&request_id.get().to_le_bytes());
    frame[20..28].copy_from_slice(&note_id.0.to_le_bytes());
    frame[28..36].copy_from_slice(&surface_id.get().to_le_bytes());
    frame[36..38].copy_from_slice(&(ATTACHMENT_COUNT as u16).to_le_bytes());
    frame[38..40].copy_from_slice(&[0, 0]);
    frame
}

#[cfg(test)]
fn decode(
    frame: &[u8; BOOTSTRAP_BYTES],
) -> Result<(NonZeroU64, NoteId, SurfaceId), BootstrapError> {
    if &frame[0..4] != MAGIC {
        return Err(BootstrapError::BadMagic);
    }
    if u16::from_le_bytes([frame[4], frame[5]]) != VERSION {
        return Err(BootstrapError::UnsupportedVersion);
    }
    if u16::from_le_bytes([frame[6], frame[7]]) as usize != BOOTSTRAP_BYTES
        || u32::from_le_bytes(frame[8..12].try_into().expect("fixed slice")) != 0
    {
        return Err(BootstrapError::BadLength);
    }
    if frame[38] != 0 || frame[39] != 0 {
        return Err(BootstrapError::NonZeroReserved);
    }
    if u16::from_le_bytes([frame[36], frame[37]]) as usize != ATTACHMENT_COUNT {
        return Err(BootstrapError::InvalidSlotCount);
    }
    let request_id = u64::from_le_bytes(frame[12..20].try_into().expect("fixed slice"));
    let note_id = u64::from_le_bytes(frame[20..28].try_into().expect("fixed slice"));
    let surface_id = u64::from_le_bytes(frame[28..36].try_into().expect("fixed slice"));
    Ok((
        NonZeroU64::new(request_id).ok_or(BootstrapError::InvalidRequest)?,
        if note_id != 0 {
            ObjectId(note_id)
        } else {
            return Err(BootstrapError::InvalidNote);
        },
        SurfaceId::new(surface_id).ok_or(BootstrapError::InvalidSurface)?,
    ))
}

#[cfg(test)]
fn send_frame_with_fds(
    mut endpoint: &UnixStream,
    frame: &[u8; BOOTSTRAP_BYTES],
    descriptors: [i32; ATTACHMENT_COUNT],
) -> Result<(), BootstrapError> {
    // c_long storage guarantees cmsghdr alignment on supported Unix targets.
    let control_bytes = unsafe {
        libc::CMSG_SPACE((ATTACHMENT_COUNT * std::mem::size_of::<i32>()) as u32) as usize
    };
    let words = control_bytes.div_ceil(std::mem::size_of::<libc::c_long>());
    let mut control = vec![0 as libc::c_long; words];
    let mut iovec = libc::iovec {
        iov_base: frame.as_ptr() as *mut libc::c_void,
        iov_len: frame.len(),
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iovec;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control_bytes as _;
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        if header.is_null() {
            return Err(BootstrapError::UnexpectedAncillary);
        }
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len =
            libc::CMSG_LEN((ATTACHMENT_COUNT * std::mem::size_of::<i32>()) as u32) as _;
        std::ptr::copy_nonoverlapping(
            descriptors.as_ptr().cast::<u8>(),
            libc::CMSG_DATA(header),
            ATTACHMENT_COUNT * std::mem::size_of::<i32>(),
        );
    }
    let sent = unsafe { libc::sendmsg(endpoint.as_raw_fd(), &message, 0) };
    if sent < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let sent = sent as usize;
    if sent == 0 {
        return Err(BootstrapError::ShortFrame);
    }
    if sent < frame.len() {
        endpoint.write_all(&frame[sent..])?;
    }
    Ok(())
}

#[cfg(test)]
fn receive_frame_with_fds(
    endpoint: &UnixStream,
) -> Result<([u8; BOOTSTRAP_BYTES], Vec<OwnedFd>), BootstrapError> {
    let control_bytes = unsafe {
        libc::CMSG_SPACE((ATTACHMENT_COUNT * std::mem::size_of::<i32>()) as u32) as usize
    };
    let words = control_bytes.div_ceil(std::mem::size_of::<libc::c_long>());
    let mut control = vec![0 as libc::c_long; words];
    let mut frame = [0u8; BOOTSTRAP_BYTES];
    let mut iovec = libc::iovec {
        iov_base: frame.as_mut_ptr().cast(),
        iov_len: frame.len(),
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iovec;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control_bytes as _;
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_CMSG_CLOEXEC | libc::MSG_WAITALL;
    #[cfg(not(target_os = "linux"))]
    let flags = libc::MSG_WAITALL;
    let received = unsafe { libc::recvmsg(endpoint.as_raw_fd(), &mut message, flags) };
    if received < 0 {
        return Err(io::Error::last_os_error().into());
    }
    if received as usize != BOOTSTRAP_BYTES {
        return Err(BootstrapError::ShortFrame);
    }
    let mut descriptors = Vec::<OwnedFd>::new();
    let mut unexpected = false;
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&message);
        while !header.is_null() {
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let data_len =
                    ((*header).cmsg_len as usize).saturating_sub(libc::CMSG_LEN(0) as usize);
                if data_len % std::mem::size_of::<i32>() != 0 {
                    unexpected = true;
                }
                let count = data_len / std::mem::size_of::<i32>();
                let data = libc::CMSG_DATA(header).cast::<i32>();
                for index in 0..count {
                    descriptors.push(OwnedFd::from_raw_fd(*data.add(index)));
                }
            } else {
                unexpected = true;
            }
            header = libc::CMSG_NXTHDR(&message, header);
        }
    }
    if message.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(BootstrapError::TruncatedAncillary);
    }
    if message.msg_flags & libc::MSG_TRUNC != 0 {
        return Err(BootstrapError::ShortFrame);
    }
    if unexpected {
        return Err(BootstrapError::UnexpectedAncillary);
    }
    if descriptors.len() != ATTACHMENT_COUNT {
        return Err(BootstrapError::WrongDescriptorCount);
    }
    #[cfg(not(target_os = "linux"))]
    for descriptor in &descriptors {
        set_cloexec(std::os::fd::AsRawFd::as_raw_fd(descriptor))
            .map_err(|_| BootstrapError::DescriptorFlags)?;
    }
    Ok((frame, descriptors))
}

fn set_cloexec(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn bootstrap_moves_exact_typed_ids_and_two_role_ordered_channels() {
        let (parent_channels, child_channels) = channel_pair().unwrap();
        let (parent_control, child_control) = UnixStream::pair().unwrap();
        let receiver = thread::spawn(move || receive_notes_bootstrap(&child_control).unwrap());
        send_notes_bootstrap(
            &parent_control,
            NonZeroU64::new(7).unwrap(),
            ObjectId(0x1122),
            SurfaceId::new(0x3344).unwrap(),
            child_channels,
        )
        .unwrap();
        let bootstrap = receiver.join().unwrap();
        assert_eq!(bootstrap.request_id.get(), 7);
        assert_eq!(bootstrap.note_id, ObjectId(0x1122));
        assert_eq!(bootstrap.surface_id.get(), 0x3344);

        let parent_input = parent_channels.input;
        parent_input.send(b"PING").unwrap();
        let mut input_probe = [0; 4];
        let mut child_input = bootstrap.channels.input;
        child_input.recv(&mut input_probe).unwrap();
        assert_eq!(&input_probe, b"PING");

        let mut child_display = bootstrap.channels.display;
        child_display.write_all(b"FRAME").unwrap();
        let mut display_probe = [0; 5];
        let mut parent_display = parent_channels.display;
        std::io::Read::read_exact(&mut parent_display, &mut display_probe).unwrap();
        assert_eq!(&display_probe, b"FRAME");
    }

    #[test]
    fn invalid_zero_surface_identity_is_rejected_before_wire_send() {
        assert!(SurfaceId::new(0).is_none());
        assert!(decode(&encode(
            NonZeroU64::new(1).unwrap(),
            ObjectId(1),
            SurfaceId::new(2).unwrap()
        ))
        .is_ok());
        let mut malformed = encode(
            NonZeroU64::new(1).unwrap(),
            ObjectId(1),
            SurfaceId::new(2).unwrap(),
        );
        malformed[38] = 1;
        assert_eq!(decode(&malformed), Err(BootstrapError::NonZeroReserved));
    }
}
