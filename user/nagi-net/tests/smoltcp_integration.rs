use libnagi::MAX_NET_FRAME_SIZE;
use nagi_net::{Device, NetError, SocketApi};

struct RecordingDevice {
    sent: usize,
}

impl Device for RecordingDevice {
    fn send(&mut self, frame: &[u8]) -> Result<(), NetError> {
        self.sent += frame.len();
        Ok(())
    }

    fn receive(&mut self, _frame: &mut [u8; MAX_NET_FRAME_SIZE]) -> Result<usize, NetError> {
        Ok(0)
    }
}

#[test]
fn non_test_smoltcp_dhcp_path_emits_through_device() {
    let mut socket_api = SocketApi::new(RecordingDevice { sent: 0 });
    assert!(socket_api.dhcp().is_err());
    assert!(socket_api.device_mut().sent > 0);
}
