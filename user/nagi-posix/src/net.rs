use nagi_net::{Device, Ipv4Address, NetError};
use nagi_pal::Network;

pub(crate) const fn supports_stream_socket(domain: i32, socket_type: i32, protocol: i32) -> bool {
    const AF_INET: i32 = 2;
    const SOCK_STREAM: i32 = 1;
    const IPPROTO_TCP: i32 = 6;

    domain == AF_INET && socket_type == SOCK_STREAM && (protocol == 0 || protocol == IPPROTO_TCP)
}

pub(crate) fn take_pending_socket_error(error: &mut i32) -> i32 {
    core::mem::replace(error, 0)
}

#[cfg(any(target_os = "nagi", test))]
pub(crate) const O_NONBLOCK: i32 = 0x0004_0000;

#[cfg(any(target_os = "nagi", test))]
pub(crate) const fn socket_is_nonblocking(status_flags: i32) -> bool {
    status_flags & O_NONBLOCK != 0
}

#[cfg(any(target_os = "nagi", test))]
pub(crate) const fn socket_status_flags(nonblocking: bool) -> i32 {
    if nonblocking {
        O_NONBLOCK
    } else {
        0
    }
}

#[cfg(any(target_os = "nagi", test))]
pub(crate) const fn network_errno(error: NetError) -> i32 {
    match error {
        NetError::Device | NetError::EntropyUnavailable => 5,
        NetError::BufferTooSmall => 90,
        NetError::DhcpTimeout
        | NetError::DnsTimeout
        | NetError::TcpTimeout
        | NetError::HttpTimeout
        | NetError::IcmpTimeout => 110,
        NetError::DnsUnavailable => 101,
        NetError::DnsFailure => 113,
        NetError::Icmp => 113,
        NetError::RouteTableFull => 105,
        NetError::WouldBlock => 11,
        NetError::Malformed | NetError::Checksum => 71,
        NetError::UnexpectedPeer | NetError::ConnectionReset => 104,
        NetError::Unsupported => 95,
    }
}

pub fn http_get<D: Device>(
    network: &mut Network<D>,
    target: Ipv4Address,
    target_port: u16,
    path: &[u8],
    expected_body: &[u8],
    response: &mut [u8],
) -> Result<usize, NetError> {
    network.http_get(target, target_port, path, expected_body, response)
}

#[cfg(test)]
mod tests {
    use super::{
        network_errno, socket_is_nonblocking, socket_status_flags, supports_stream_socket,
        take_pending_socket_error, O_NONBLOCK,
    };
    use nagi_net::NetError;

    #[test]
    fn ipv4_stream_sockets_accept_default_and_explicit_tcp_protocols() {
        assert!(supports_stream_socket(2, 1, 0));
        assert!(supports_stream_socket(2, 1, 6));
        assert!(!supports_stream_socket(2, 1, 17));
        assert!(!supports_stream_socket(10, 1, 6));
        assert!(!supports_stream_socket(2, 2, 6));
    }

    #[test]
    fn so_error_returns_and_clears_a_pending_socket_error() {
        let mut pending = 111;
        assert_eq!(take_pending_socket_error(&mut pending), 111);
        assert_eq!(pending, 0);
        assert_eq!(take_pending_socket_error(&mut pending), 0);
    }

    #[test]
    fn nonblocking_status_flag_round_trips_without_inventing_other_flags() {
        assert!(!socket_is_nonblocking(0));
        assert!(socket_is_nonblocking(O_NONBLOCK));
        assert!(socket_is_nonblocking(O_NONBLOCK | 0x20));
        assert_eq!(socket_status_flags(false), 0);
        assert_eq!(socket_status_flags(true), O_NONBLOCK);
    }

    #[test]
    fn network_failures_keep_timeout_dns_reset_and_would_block_distinct() {
        assert_eq!(network_errno(NetError::DnsFailure), 113);
        assert_eq!(network_errno(NetError::DnsTimeout), 110);
        assert_eq!(network_errno(NetError::TcpTimeout), 110);
        assert_eq!(network_errno(NetError::EntropyUnavailable), 5);
        assert_eq!(network_errno(NetError::ConnectionReset), 104);
        assert_eq!(network_errno(NetError::WouldBlock), 11);
    }
}
