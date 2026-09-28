use nagi_net::{Device, Ipv4Address, NetError};
use nagi_pal::Network;

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
    use super::{socket_is_nonblocking, socket_status_flags, O_NONBLOCK};

    #[test]
    fn nonblocking_status_flag_round_trips_without_inventing_other_flags() {
        assert!(!socket_is_nonblocking(0));
        assert!(socket_is_nonblocking(O_NONBLOCK));
        assert!(socket_is_nonblocking(O_NONBLOCK | 0x20));
        assert_eq!(socket_status_flags(false), 0);
        assert_eq!(socket_status_flags(true), O_NONBLOCK);
    }
}
