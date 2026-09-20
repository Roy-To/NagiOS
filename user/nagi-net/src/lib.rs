#![no_std]

use libnagi::MAX_NET_FRAME_SIZE;

mod smoltcp_stack;

pub use smoltcp_stack::SocketApi;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv4Address(pub [u8; 4]);

impl Ipv4Address {
    pub const fn new(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }

    pub const fn octets(self) -> [u8; 4] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetError {
    Device,
    BufferTooSmall,
    DhcpTimeout,
    DnsUnavailable,
    DnsTimeout,
    Icmp,
    IcmpTimeout,
    RouteTableFull,
    TcpTimeout,
    HttpTimeout,
    Malformed,
    Checksum,
    UnexpectedPeer,
    ConnectionReset,
    Unsupported,
}

/// Capability-scoped raw Ethernet device boundary.
///
/// Implementations may only exchange frames through the capability they were
/// handed by the kernel. Higher-level protocols belong to `nagi-net` and use
/// smoltcp through the adapter in `smoltcp_stack`.
pub trait Device {
    fn send(&mut self, frame: &[u8]) -> Result<(), NetError>;
    fn receive(&mut self, frame: &mut [u8; MAX_NET_FRAME_SIZE]) -> Result<usize, NetError>;
}

pub struct SyscallDevice {
    capability: u64,
}

impl SyscallDevice {
    pub const fn new(capability: u64) -> Self {
        Self { capability }
    }
}

impl Device for SyscallDevice {
    fn send(&mut self, frame: &[u8]) -> Result<(), NetError> {
        if libnagi::net_send(self.capability, frame) {
            Ok(())
        } else {
            Err(NetError::Device)
        }
    }

    fn receive(&mut self, frame: &mut [u8; MAX_NET_FRAME_SIZE]) -> Result<usize, NetError> {
        Ok(libnagi::net_receive(self.capability, frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptyDevice;

    impl Device for EmptyDevice {
        fn send(&mut self, _frame: &[u8]) -> Result<(), NetError> {
            Ok(())
        }

        fn receive(&mut self, _frame: &mut [u8; MAX_NET_FRAME_SIZE]) -> Result<usize, NetError> {
            Ok(0)
        }
    }

    #[test]
    fn stack_uses_a_capability_scoped_device() {
        let mut stack = SocketApi::new(EmptyDevice);
        assert!(stack.device_mut().send(&[]).is_ok());
    }
}
