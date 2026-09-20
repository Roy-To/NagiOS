use nagi_net::{Device, Ipv4Address, NetError};
use nagi_pal::Network;

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
