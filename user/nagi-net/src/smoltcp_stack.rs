use core::array;
#[cfg(all(feature = "m18-browser-network", target_os = "nagi"))]
use core::sync::atomic::{AtomicUsize, Ordering};

use libnagi::MAX_NET_FRAME_SIZE;
use smoltcp::iface::{
    Config as InterfaceConfig, Interface, PollIngressSingleResult, SocketHandle, SocketSet,
    SocketStorage,
};
use smoltcp::phy::{
    ChecksumCapabilities, Device as PhyDevice, DeviceCapabilities, Medium, RxToken, TxToken,
};
use smoltcp::socket::{dhcpv4, dns, icmp, tcp};
use smoltcp::storage::{PacketBuffer, PacketMetadata};
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{
    EthernetAddress, HardwareAddress, Icmpv4Packet, Icmpv4Repr, IpAddress, IpCidr,
    Ipv4Address as SmolIpv4,
};

use crate::{Device, Ipv4Address, NetError, TcpConnectionId};

const POLL_BUDGET: usize = 1_000_000;
#[cfg(not(feature = "m18-browser-network"))]
const DNS_POLL_BUDGET: usize = 200;
#[cfg(feature = "m18-browser-network")]
const DNS_POLL_BUDGET: usize = 20_000;
const DHCP_TIMEOUT: Duration = Duration::from_secs(30);
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SOCKET_BUFFER_SIZE: usize = 2048;
#[cfg(feature = "m18-browser-network")]
const TCP_CONNECTION_CAPACITY: usize = 4;
#[cfg(not(feature = "m18-browser-network"))]
const TCP_CONNECTION_CAPACITY: usize = 1;
const LOCAL_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x15];
#[cfg(all(feature = "m18-browser-network", target_os = "nagi"))]
static M18_DNS_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(all(feature = "m18-browser-network", target_os = "nagi"))]
static M18_DNS_POLL_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn trace_m18_dns(message: &[u8]) {
    #[cfg(all(feature = "m18-browser-network", target_os = "nagi"))]
    if M18_DNS_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 24 {
        let _ = libnagi::console_write(message);
    }
    #[cfg(not(all(feature = "m18-browser-network", target_os = "nagi")))]
    let _ = message;
}

#[inline]
fn trace_m18_dns_poll(message: &[u8]) {
    #[cfg(all(feature = "m18-browser-network", target_os = "nagi"))]
    if M18_DNS_POLL_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 128 {
        let _ = libnagi::console_write(message);
    }
    #[cfg(not(all(feature = "m18-browser-network", target_os = "nagi")))]
    let _ = message;
}

#[inline]
fn trace_m18_dns_iteration(iteration: usize, stage: &[u8]) {
    let ordinal: &[u8] = match iteration {
        3 => b"fourth",
        4 => b"fifth",
        5 => b"sixth",
        6 => b"seventh",
        7 => b"eighth",
        8 => b"ninth",
        9 => b"tenth",
        _ => return,
    };
    let prefix = b"Nagi M18 smoltcp DNS ";
    let mut line = [0_u8; 96];
    let mut length = prefix.len();
    line[..length].copy_from_slice(prefix);
    line[length..length + ordinal.len()].copy_from_slice(ordinal);
    length += ordinal.len();
    line[length..length + stage.len()].copy_from_slice(stage);
    length += stage.len();
    trace_m18_dns_poll(&line[..length]);
}

// Nagi's initial user stack is intentionally small. These buffers belong to
// this single-threaded network service and keep packet storage out of the
// service call stack without using a host allocator.
static mut PHY_RX_FRAME: [u8; MAX_NET_FRAME_SIZE] = [0; MAX_NET_FRAME_SIZE];
static mut PHY_TX_FRAME: [u8; MAX_NET_FRAME_SIZE] = [0; MAX_NET_FRAME_SIZE];
static mut TCP_RX_STORAGE: [[u8; SOCKET_BUFFER_SIZE]; TCP_CONNECTION_CAPACITY] =
    [[0; SOCKET_BUFFER_SIZE]; TCP_CONNECTION_CAPACITY];
static mut TCP_TX_STORAGE: [[u8; SOCKET_BUFFER_SIZE]; TCP_CONNECTION_CAPACITY] =
    [[0; SOCKET_BUFFER_SIZE]; TCP_CONNECTION_CAPACITY];
static mut TCP_SOCKET_STORAGE: [SocketStorage<'static>; TCP_CONNECTION_CAPACITY] =
    [const { SocketStorage::EMPTY }; TCP_CONNECTION_CAPACITY];

pub const SOCKET_READY_READ: i16 = 0x0001;
pub const SOCKET_READY_WRITE: i16 = 0x0004;
pub const SOCKET_READY_ERROR: i16 = 0x0008;

/// A smoltcp physical device adapter over Nagi's capability-checked raw-frame ABI.
///
/// The adapter is deliberately the only place where `nagi-net` calls the raw
/// frame device. All IP, ARP, DHCP, ICMP, UDP, DNS and TCP behavior is owned by
/// smoltcp in user space.
pub struct NagiPhyDevice<'a, D: Device> {
    device: &'a mut D,
}

impl<'a, D: Device> NagiPhyDevice<'a, D> {
    pub fn new(device: &'a mut D) -> Self {
        Self { device }
    }
}

pub struct NagiRxToken<'a> {
    frame: &'a [u8],
}

pub struct NagiTxToken<'a, D: Device> {
    device: &'a mut D,
}

impl RxToken for NagiRxToken<'_> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(self.frame)
    }
}

impl<D: Device> TxToken for NagiTxToken<'_, D> {
    fn consume<R, F>(self, length: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let writable_length = length.min(MAX_NET_FRAME_SIZE);
        let result;
        unsafe {
            let frame = core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(PHY_TX_FRAME).cast::<u8>(),
                writable_length,
            );
            result = f(frame);
            let _ = self.device.send(frame);
        }
        result
    }
}

impl<D: Device> PhyDevice for NagiPhyDevice<'_, D> {
    type RxToken<'a>
        = NagiRxToken<'a>
    where
        Self: 'a;
    type TxToken<'a>
        = NagiTxToken<'a, D>
    where
        Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let length;
        unsafe {
            let frame = &mut *core::ptr::addr_of_mut!(PHY_RX_FRAME);
            length = self.device.receive(frame).ok()?;
        }
        if length == 0 || length > MAX_NET_FRAME_SIZE {
            return None;
        }
        let frame = unsafe {
            core::slice::from_raw_parts(core::ptr::addr_of!(PHY_RX_FRAME).cast::<u8>(), length)
        };
        Some((
            NagiRxToken { frame },
            NagiTxToken {
                device: self.device,
            },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(NagiTxToken {
            device: self.device,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.medium = Medium::Ethernet;
        capabilities.max_transmission_unit = MAX_NET_FRAME_SIZE;
        capabilities.max_burst_size = Some(1);
        capabilities
    }
}

#[derive(Clone, Copy)]
struct NetworkConfig {
    address: smoltcp::wire::Ipv4Cidr,
    router: Option<SmolIpv4>,
    dns: Option<SmolIpv4>,
}

fn dns_a_result(addresses: &[IpAddress]) -> Result<Ipv4Address, NetError> {
    addresses
        .iter()
        .next()
        .map(|address| {
            let IpAddress::Ipv4(address) = address;
            from_smol_ipv4(*address)
        })
        .ok_or(NetError::DnsFailure)
}

/// User-space network stack backed by smoltcp and Nagi's raw VirtIO capability.
struct SmoltcpStack<D> {
    device: D,
    next_port: u16,
    network_config: Option<NetworkConfig>,
    tcp_interface: Option<Interface>,
    tcp_sockets: Option<SocketSet<'static>>,
    tcp_handles: [Option<SocketHandle>; TCP_CONNECTION_CAPACITY],
}

impl<D: Device> SmoltcpStack<D> {
    pub const fn new(device: D) -> Self {
        Self {
            device,
            next_port: 40_000,
            network_config: None,
            tcp_interface: None,
            tcp_sockets: None,
            tcp_handles: [None; TCP_CONNECTION_CAPACITY],
        }
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    /// Obtain IPv4 configuration from the guest network using DHCPv4.
    pub fn dhcp(&mut self) -> Result<Ipv4Address, NetError> {
        Ok(from_smol_ipv4(self.ensure_dhcp()?.address.address()))
    }

    pub fn dhcp_gateway(&mut self) -> Result<Ipv4Address, NetError> {
        self.ensure_dhcp()?
            .router
            .map(from_smol_ipv4)
            .ok_or(NetError::DnsUnavailable)
    }

    /// Resolve an A record through the DNS server received from DHCP.
    pub fn resolve_ipv4(&mut self, name: &str) -> Result<Ipv4Address, NetError> {
        #[cfg(all(feature = "m18-browser-network", target_os = "nagi"))]
        {
            M18_DNS_TRACE_COUNT.store(0, Ordering::Relaxed);
            M18_DNS_POLL_TRACE_COUNT.store(0, Ordering::Relaxed);
        }
        let config = self.ensure_dhcp()?;
        let mut phy = NagiPhyDevice::new(&mut self.device);
        let mut interface = new_interface(&mut phy)?;
        apply_network_config(&mut interface, config)?;
        let mut storage: [SocketStorage<'_>; 8] = array::from_fn(|_| SocketStorage::EMPTY);
        let mut sockets = SocketSet::new(&mut storage[..]);
        let server = config.dns.ok_or(NetError::DnsUnavailable)?;
        let mut queries: [Option<dns::DnsQuery>; 1] = array::from_fn(|_| None);
        let dns_socket = dns::Socket::new(&[IpAddress::Ipv4(server)], &mut queries[..]);
        let dns_handle = sockets.add(dns_socket);
        trace_m18_dns(b"Nagi M18 smoltcp DNS query setup started\r\n");
        let query = sockets
            .get_mut::<dns::Socket>(dns_handle)
            .start_query(interface.context(), name, smoltcp::wire::DnsQueryType::A)
            .map_err(|_| NetError::Malformed)?;
        trace_m18_dns(b"Nagi M18 smoltcp DNS query setup completed\r\n");
        #[cfg(feature = "m18-browser-network")]
        let started = now();
        for iteration in 0..DNS_POLL_BUDGET {
            if iteration == 0 {
                trace_m18_dns(b"Nagi M18 smoltcp DNS first poll entered\r\n");
            }
            let timestamp = now();
            if iteration == 1 {
                trace_m18_dns_poll(b"Nagi M18 smoltcp DNS second poll entered\r\n");
            }
            if iteration == 2 {
                trace_m18_dns_poll(b"Nagi M18 smoltcp DNS third ingress entered\r\n");
            }
            if (3..=9).contains(&iteration) {
                trace_m18_dns_iteration(iteration, b" ingress entered\r\n");
            }
            let ingress = interface.poll_ingress_single(timestamp, &mut phy, &mut sockets);
            if iteration == 2 {
                trace_m18_dns_poll(b"Nagi M18 smoltcp DNS third ingress returned\r\n");
                trace_m18_dns_poll(b"Nagi M18 smoltcp DNS third egress entered\r\n");
            }
            if (3..=9).contains(&iteration) {
                trace_m18_dns_iteration(iteration, b" ingress returned\r\n");
                trace_m18_dns_iteration(iteration, b" egress entered\r\n");
            }
            let _ = interface.poll_egress(timestamp, &mut phy, &mut sockets);
            if iteration == 2 {
                trace_m18_dns_poll(b"Nagi M18 smoltcp DNS third egress returned\r\n");
            }
            if (3..=9).contains(&iteration) {
                trace_m18_dns_iteration(iteration, b" egress returned\r\n");
            }
            if iteration == 1 {
                trace_m18_dns_poll(b"Nagi M18 smoltcp DNS second poll returned\r\n");
            }
            if iteration != 0 && iteration.is_multiple_of(256) {
                trace_m18_dns_poll(
                    if matches!(
                        ingress,
                        PollIngressSingleResult::PacketProcessed
                            | PollIngressSingleResult::SocketStateChanged
                    ) {
                        b"Nagi M18 smoltcp DNS poll received a frame\r\n"
                    } else {
                        b"Nagi M18 smoltcp DNS poll had no frame\r\n"
                    },
                );
            }
            if iteration == 0 {
                trace_m18_dns(b"Nagi M18 smoltcp DNS first poll returned\r\n");
            }
            match sockets
                .get_mut::<dns::Socket>(dns_handle)
                .get_query_result(query)
            {
                Ok(addresses) => {
                    trace_m18_dns(b"Nagi M18 smoltcp DNS answer received\r\n");
                    return dns_a_result(&addresses);
                }
                Err(dns::GetQueryResultError::Pending) => {
                    if iteration == 0 {
                        trace_m18_dns(b"Nagi M18 smoltcp DNS answer pending\r\n");
                    }
                    if (3..=9).contains(&iteration) {
                        trace_m18_dns_iteration(iteration, b" answer pending\r\n");
                    }
                    #[cfg(feature = "m18-browser-network")]
                    {
                        // Keep the Tokio worker cooperative while its synchronous
                        // resolver polls the guest VirtIO device. A timed sleep
                        // parks this worker on the coarse kernel clock and can
                        // prevent further network polling on the M18 path.
                        if iteration == 1 {
                            trace_m18_dns_poll(b"Nagi M18 smoltcp DNS second answer pending\r\n");
                        }
                        if iteration == 2 {
                            trace_m18_dns_poll(b"Nagi M18 smoltcp DNS third answer pending\r\n");
                        }
                        if iteration != 0 && iteration.is_multiple_of(256) {
                            trace_m18_dns_poll(b"Nagi M18 smoltcp DNS poll loop progressed\r\n");
                        }
                        let elapsed = now() - started;
                        if elapsed >= Duration::from_secs(2) {
                            if iteration == 1 {
                                trace_m18_dns_poll(
                                    b"Nagi M18 smoltcp DNS second poll timed out\r\n",
                                );
                            }
                            if iteration == 2 {
                                trace_m18_dns_poll(
                                    b"Nagi M18 smoltcp DNS third poll timed out\r\n",
                                );
                            }
                            return Err(NetError::DnsTimeout);
                        }
                        if iteration == 1 {
                            trace_m18_dns_poll(b"Nagi M18 smoltcp DNS second yield entered\r\n");
                        }
                        if iteration == 2 {
                            trace_m18_dns_poll(b"Nagi M18 smoltcp DNS third yield entered\r\n");
                        }
                        if (3..=9).contains(&iteration) {
                            trace_m18_dns_iteration(iteration, b" yield entered\r\n");
                        }
                        if iteration == 0 {
                            trace_m18_dns(b"Nagi M18 smoltcp DNS worker yield entered\r\n");
                        }
                        let yielded = libnagi::thread_yield();
                        if iteration == 1 {
                            trace_m18_dns_poll(if yielded {
                                b"Nagi M18 smoltcp DNS second yield returned\r\n"
                            } else {
                                b"Nagi M18 smoltcp DNS second yield failed\r\n"
                            });
                        }
                        if iteration == 2 {
                            trace_m18_dns_poll(if yielded {
                                b"Nagi M18 smoltcp DNS third yield returned\r\n"
                            } else {
                                b"Nagi M18 smoltcp DNS third yield failed\r\n"
                            });
                        }
                        if (3..=9).contains(&iteration) {
                            trace_m18_dns_iteration(
                                iteration,
                                if yielded {
                                    b" yield returned\r\n"
                                } else {
                                    b" yield failed\r\n"
                                },
                            );
                        }
                        if iteration == 0 {
                            trace_m18_dns(if yielded {
                                b"Nagi M18 smoltcp DNS worker yield returned\r\n"
                            } else {
                                b"Nagi M18 smoltcp DNS worker yield failed\r\n"
                            });
                        }
                        if !yielded {
                            return Err(NetError::DnsTimeout);
                        }
                    }
                    #[cfg(not(feature = "m18-browser-network"))]
                    {
                        if !libnagi::sleep_ns(1_000_000) {
                            return Err(NetError::DnsTimeout);
                        }
                    }
                }
                Err(dns::GetQueryResultError::Failed) => {
                    trace_m18_dns(b"Nagi M18 smoltcp DNS query failed\r\n");
                    return Err(NetError::DnsFailure);
                }
            }
        }
        trace_m18_dns(b"Nagi M18 smoltcp DNS poll budget exhausted\r\n");
        Err(NetError::DnsTimeout)
    }

    /// Perform a TCP HTTP request through smoltcp after DHCP configuration.
    pub fn http_get(
        &mut self,
        target: Ipv4Address,
        target_port: u16,
        path: &[u8],
        expected_body: &[u8],
        response: &mut [u8],
    ) -> Result<usize, NetError> {
        let mut request = [0_u8; 512];
        let request_length = http_request(path, &mut request)?;
        let connection = self.tcp_connect(target, target_port)?;
        if let Err(error) = self.tcp_send(connection, &request[..request_length]) {
            let _ = self.tcp_close(connection);
            return Err(error);
        }

        let mut response_length = 0;
        let mut expected_found = false;
        let result = 'receive: loop {
            if response_length == response.len() {
                break 'receive Err(NetError::BufferTooSmall);
            }
            match self.tcp_receive(connection, &mut response[response_length..]) {
                Ok(0) => {
                    break 'receive if expected_found {
                        Ok(response_length)
                    } else {
                        Err(NetError::ConnectionReset)
                    };
                }
                Ok(count) => {
                    response_length = response_length
                        .checked_add(count)
                        .ok_or(NetError::BufferTooSmall)?;
                    expected_found |= contains_http_ok(response, response_length, expected_body);
                }
                Err(NetError::TcpTimeout) if expected_found => {
                    break 'receive Ok(response_length);
                }
                Err(error) => break 'receive Err(error),
            }
        };
        let close_result = self.tcp_close(connection);
        match result {
            Ok(length) => {
                close_result?;
                Ok(length)
            }
            Err(error) => {
                let _ = close_result;
                Err(error)
            }
        }
    }

    /// Open a bounded TCP connection and retain its smoltcp state in the
    /// user-space network service. M18 provides a small fixed socket pool;
    /// earlier milestones retain their one-socket capacity.
    pub fn tcp_connect(
        &mut self,
        target: Ipv4Address,
        target_port: u16,
    ) -> Result<TcpConnectionId, NetError> {
        let slot = first_free_tcp_slot(&self.tcp_handles).ok_or(NetError::Unsupported)?;
        let network_config = self.ensure_dhcp()?;
        let local_port = self.next_port;
        self.next_port = self.next_port.wrapping_add(1).max(40_000);
        if self.tcp_interface.is_none() {
            let mut phy = NagiPhyDevice::new(&mut self.device);
            let mut interface = new_interface(&mut phy)?;
            apply_network_config(&mut interface, network_config)?;
            self.tcp_interface = Some(interface);
        }

        if self.tcp_sockets.is_none() {
            let storage = unsafe {
                core::slice::from_raw_parts_mut(
                    core::ptr::addr_of_mut!(TCP_SOCKET_STORAGE).cast::<SocketStorage<'static>>(),
                    TCP_CONNECTION_CAPACITY,
                )
            };
            self.tcp_sockets = Some(SocketSet::new(storage));
        }
        let tcp_socket = unsafe {
            let rx = core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(TCP_RX_STORAGE)
                    .cast::<u8>()
                    .add(slot * SOCKET_BUFFER_SIZE),
                SOCKET_BUFFER_SIZE,
            );
            let tx = core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(TCP_TX_STORAGE)
                    .cast::<u8>()
                    .add(slot * SOCKET_BUFFER_SIZE),
                SOCKET_BUFFER_SIZE,
            );
            tcp::Socket::new(tcp::SocketBuffer::new(rx), tcp::SocketBuffer::new(tx))
        };
        let interface = self
            .tcp_interface
            .as_mut()
            .ok_or(NetError::ConnectionReset)?;
        let sockets = self.tcp_sockets.as_mut().ok_or(NetError::ConnectionReset)?;
        let handle = sockets.add(tcp_socket);
        if sockets
            .get_mut::<tcp::Socket>(handle)
            .connect(
                interface.context(),
                (IpAddress::Ipv4(to_smol_ipv4(target)), target_port),
                local_port,
            )
            .is_err()
        {
            let _ = sockets.remove(handle);
            return Err(NetError::ConnectionReset);
        }

        let started = now();
        for attempt in 0..POLL_BUDGET {
            let mut phy = NagiPhyDevice::new(&mut self.device);
            poll_once(interface, &mut phy, sockets, now());
            let socket = sockets.get::<tcp::Socket>(handle);
            if socket.may_send() {
                self.tcp_handles[slot] = Some(handle);
                return Ok(TcpConnectionId::from_index(slot));
            }
            if !socket.is_open() {
                let _ = sockets.remove(handle);
                return Err(NetError::ConnectionReset);
            }
            if now() - started >= TCP_CONNECT_TIMEOUT {
                break;
            }
            if attempt % 32 == 31 && !libnagi::sleep_ns(1_000_000) {
                break;
            }
        }
        let _ = sockets.remove(handle);
        Err(NetError::TcpTimeout)
    }

    /// Send bytes on the retained TCP stream, polling smoltcp until the
    /// bounded transmit buffer accepts them or the connection fails.
    fn tcp_handle(&self, connection: TcpConnectionId) -> Result<SocketHandle, NetError> {
        self.tcp_handles
            .get(connection.index())
            .and_then(|handle| *handle)
            .ok_or(NetError::ConnectionReset)
    }

    pub fn tcp_send(
        &mut self,
        connection: TcpConnectionId,
        data: &[u8],
    ) -> Result<usize, NetError> {
        let handle = self.tcp_handle(connection)?;
        let (Some(interface), Some(sockets)) =
            (self.tcp_interface.as_mut(), self.tcp_sockets.as_mut())
        else {
            return Err(NetError::ConnectionReset);
        };
        let mut sent = 0;
        while sent < data.len() {
            {
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                if socket.can_send() {
                    let count = socket
                        .send_slice(&data[sent..])
                        .map_err(|_| NetError::ConnectionReset)?;
                    sent = sent.saturating_add(count);
                    if count == 0 {
                        return Err(NetError::ConnectionReset);
                    }
                } else if !socket.is_open() {
                    return Err(NetError::ConnectionReset);
                }
            }
            let mut phy = NagiPhyDevice::new(&mut self.device);
            poll_once(interface, &mut phy, sockets, now());
            if sent < data.len() {
                let socket = sockets.get::<tcp::Socket>(handle);
                if !socket.is_open() && !socket.may_send() {
                    return Err(NetError::ConnectionReset);
                }
            }
        }
        Ok(sent)
    }

    /// Make one bounded nonblocking write attempt on the retained stream.
    ///
    /// This is the operation used by POSIX sockets after `O_NONBLOCK` has been
    /// set. It never waits for additional TCP send-buffer capacity: callers
    /// either receive the number of bytes accepted in this poll step or
    /// `WouldBlock` when the send buffer is full.
    pub fn tcp_try_send(
        &mut self,
        connection: TcpConnectionId,
        data: &[u8],
    ) -> Result<usize, NetError> {
        if data.is_empty() {
            return Ok(0);
        }
        let handle = self.tcp_handle(connection)?;
        let (Some(interface), Some(sockets)) =
            (self.tcp_interface.as_mut(), self.tcp_sockets.as_mut())
        else {
            return Err(NetError::ConnectionReset);
        };
        let count = {
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if !socket.can_send() {
                return if socket.is_open() {
                    Err(NetError::WouldBlock)
                } else {
                    Err(NetError::ConnectionReset)
                };
            }
            socket
                .send_slice(data)
                .map_err(|_| NetError::ConnectionReset)?
        };
        if count == 0 {
            return Err(NetError::WouldBlock);
        }
        let mut phy = NagiPhyDevice::new(&mut self.device);
        poll_once(interface, &mut phy, sockets, now());
        Ok(count)
    }

    /// Close only the transmit half of the retained TCP stream. This is the
    /// user-space implementation boundary for POSIX `shutdown(SHUT_WR)`.
    pub fn tcp_shutdown_write(&mut self, connection: TcpConnectionId) -> Result<(), NetError> {
        let handle = self.tcp_handle(connection)?;
        let Some(sockets) = self.tcp_sockets.as_mut() else {
            return Err(NetError::ConnectionReset);
        };
        sockets.get_mut::<tcp::Socket>(handle).close();
        Ok(())
    }

    /// Apply TCP_NODELAY to the retained stream. The setting is owned by
    /// smoltcp rather than treated as a successful no-op.
    pub fn tcp_set_nagle(
        &mut self,
        connection: TcpConnectionId,
        enabled: bool,
    ) -> Result<(), NetError> {
        let handle = self.tcp_handle(connection)?;
        let Some(sockets) = self.tcp_sockets.as_mut() else {
            return Err(NetError::ConnectionReset);
        };
        sockets
            .get_mut::<tcp::Socket>(handle)
            .set_nagle_enabled(enabled);
        Ok(())
    }

    /// Apply a bounded TCP timeout to the retained stream.
    pub fn tcp_set_timeout(
        &mut self,
        connection: TcpConnectionId,
        timeout: Option<(u64, u32)>,
    ) -> Result<(), NetError> {
        let handle = self.tcp_handle(connection)?;
        let Some(sockets) = self.tcp_sockets.as_mut() else {
            return Err(NetError::ConnectionReset);
        };
        let timeout = timeout.map(|(seconds, microseconds)| {
            Duration::from_micros(
                seconds
                    .saturating_mul(1_000_000)
                    .saturating_add(u64::from(microseconds)),
            )
        });
        sockets.get_mut::<tcp::Socket>(handle).set_timeout(timeout);
        Ok(())
    }

    /// Receive available bytes from the retained TCP stream. This is a
    /// bounded polling operation; callers use `tcp_ready` for readiness.
    pub fn tcp_receive(
        &mut self,
        connection: TcpConnectionId,
        buffer: &mut [u8],
    ) -> Result<usize, NetError> {
        let handle = self.tcp_handle(connection)?;
        let (Some(interface), Some(sockets)) =
            (self.tcp_interface.as_mut(), self.tcp_sockets.as_mut())
        else {
            return Err(NetError::ConnectionReset);
        };
        for _ in 0..POLL_BUDGET {
            let mut phy = NagiPhyDevice::new(&mut self.device);
            poll_once(interface, &mut phy, sockets, now());
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if socket.can_recv() {
                return socket
                    .recv_slice(buffer)
                    .map_err(|_| NetError::ConnectionReset);
            }
            if !socket.may_recv() {
                return Ok(0);
            }
        }
        Err(NetError::TcpTimeout)
    }

    /// Poll the TCP stream once and receive currently queued bytes without
    /// waiting for a future packet. A still-open stream with no data returns
    /// `WouldBlock`.
    pub fn tcp_try_receive(
        &mut self,
        connection: TcpConnectionId,
        buffer: &mut [u8],
    ) -> Result<usize, NetError> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let handle = self.tcp_handle(connection)?;
        let (Some(interface), Some(sockets)) =
            (self.tcp_interface.as_mut(), self.tcp_sockets.as_mut())
        else {
            return Err(NetError::ConnectionReset);
        };
        let mut phy = NagiPhyDevice::new(&mut self.device);
        poll_once(interface, &mut phy, sockets, now());
        let socket = sockets.get_mut::<tcp::Socket>(handle);
        if socket.can_recv() {
            return socket
                .recv_slice(buffer)
                .map_err(|_| NetError::ConnectionReset);
        }
        if !socket.may_recv() {
            return Ok(0);
        }
        Err(NetError::WouldBlock)
    }

    /// Report readiness for the retained TCP stream without exposing smoltcp
    /// handles to POSIX callers.
    pub fn tcp_ready(
        &mut self,
        connection: TcpConnectionId,
        requested: i16,
    ) -> Result<i16, NetError> {
        let handle = self.tcp_handle(connection)?;
        let (Some(interface), Some(sockets)) =
            (self.tcp_interface.as_mut(), self.tcp_sockets.as_mut())
        else {
            return Err(NetError::ConnectionReset);
        };
        let mut phy = NagiPhyDevice::new(&mut self.device);
        poll_once(interface, &mut phy, sockets, now());
        let socket = sockets.get::<tcp::Socket>(handle);
        let mut ready = 0;
        if requested & SOCKET_READY_READ != 0 && (socket.can_recv() || !socket.may_recv()) {
            ready |= SOCKET_READY_READ;
        }
        if requested & SOCKET_READY_WRITE != 0 && socket.can_send() {
            ready |= SOCKET_READY_WRITE;
        }
        if !socket.is_open() {
            ready |= SOCKET_READY_ERROR;
        }
        Ok(ready)
    }

    pub fn tcp_close(&mut self, connection: TcpConnectionId) -> Result<(), NetError> {
        let handle = self.tcp_handle(connection)?;
        let slot = connection.index();
        #[cfg(feature = "m18-browser-network")]
        {
            let (Some(interface), Some(sockets)) =
                (self.tcp_interface.as_mut(), self.tcp_sockets.as_mut())
            else {
                return Err(NetError::ConnectionReset);
            };
            sockets.get_mut::<tcp::Socket>(handle).abort();
            // Keep the shared interface and socket set alive for other browser
            // streams. Poll through smoltcp so queued frames for those streams
            // remain routed to their own socket instead of being drained raw.
            for _ in 0..64 {
                let mut phy = NagiPhyDevice::new(&mut self.device);
                poll_once(interface, &mut phy, sockets, now());
            }
            let _ = sockets.remove(handle);
            self.tcp_handles[slot] = None;
        }
        #[cfg(not(feature = "m18-browser-network"))]
        {
            let (Some(_interface), Some(mut sockets)) =
                (self.tcp_interface.take(), self.tcp_sockets.take())
            else {
                return Err(NetError::ConnectionReset);
            };
            sockets.get_mut::<tcp::Socket>(handle).abort();
            // The one-socket M17 service drains queued frames before a later
            // protocol operation builds a fresh socket set.
            let mut phy = NagiPhyDevice::new(&mut self.device);
            for _ in 0..64 {
                if phy.receive(now()).is_none() {
                    break;
                }
            }
            let _ = sockets.remove(handle);
            self.tcp_handles[slot] = None;
            reset_tcp_storage();
        }
        Ok(())
    }

    /// Return the local endpoint selected by smoltcp for the retained TCP
    /// stream. The POSIX adapter uses this to implement `getsockname` without
    /// inventing a port or consulting a host socket table.
    pub fn tcp_local_name(
        &self,
        connection: TcpConnectionId,
    ) -> Result<(Ipv4Address, u16), NetError> {
        let config = self.network_config.ok_or(NetError::ConnectionReset)?;
        let handle = self.tcp_handle(connection)?;
        let sockets = self.tcp_sockets.as_ref().ok_or(NetError::ConnectionReset)?;
        let endpoint = sockets
            .get::<tcp::Socket>(handle)
            .local_endpoint()
            .ok_or(NetError::ConnectionReset)?;
        if !matches!(endpoint.addr, IpAddress::Ipv4(_)) {
            return Err(NetError::Unsupported);
        }
        Ok((from_smol_ipv4(config.address.address()), endpoint.port))
    }

    /// Send one ICMP echo request and require an echo reply from the target.
    pub fn icmp_echo(&mut self, target: Ipv4Address) -> Result<(), NetError> {
        let network_config = self.ensure_dhcp()?;
        let mut phy = NagiPhyDevice::new(&mut self.device);
        let mut interface = new_interface(&mut phy)?;
        apply_network_config(&mut interface, network_config)?;
        let mut storage: [SocketStorage<'_>; 8] = array::from_fn(|_| SocketStorage::EMPTY);
        let mut sockets = SocketSet::new(&mut storage[..]);
        let mut rx_meta = [PacketMetadata::EMPTY; 2];
        let mut tx_meta = [PacketMetadata::EMPTY; 2];
        let mut rx_data = [0_u8; 128];
        let mut tx_data = [0_u8; 128];
        let mut socket = icmp::Socket::new(
            PacketBuffer::new(&mut rx_meta[..], &mut rx_data[..]),
            PacketBuffer::new(&mut tx_meta[..], &mut tx_data[..]),
        );
        socket
            .bind(icmp::Endpoint::Ident(0x4e47))
            .map_err(|_| NetError::Icmp)?;
        let handle = sockets.add(socket);
        let mut request = [0_u8; 64];
        let request_length = encode_icmp_echo_request(&mut request, 0x4e47, 1, b"NAGI-ICMP")?;
        sockets
            .get_mut::<icmp::Socket>(handle)
            .send_slice(
                &request[..request_length],
                IpAddress::Ipv4(to_smol_ipv4(target)),
            )
            .map_err(|_| NetError::Icmp)?;
        for _ in 0..POLL_BUDGET {
            poll_once(&mut interface, &mut phy, &mut sockets, now());
            let socket = sockets.get_mut::<icmp::Socket>(handle);
            if socket.can_recv() {
                let mut data = [0_u8; 64];
                let (length, source) = socket.recv_slice(&mut data).map_err(|_| NetError::Icmp)?;
                if source == IpAddress::Ipv4(to_smol_ipv4(target)) {
                    let packet =
                        Icmpv4Packet::new_checked(&data[..length]).map_err(|_| NetError::Icmp)?;
                    let representation =
                        Icmpv4Repr::parse(&packet, &ChecksumCapabilities::default())
                            .map_err(|_| NetError::Icmp)?;
                    if representation
                        == (Icmpv4Repr::EchoReply {
                            ident: 0x4e47,
                            seq_no: 1,
                            data: b"NAGI-ICMP",
                        })
                    {
                        return Ok(());
                    }
                }
            }
        }
        Err(NetError::IcmpTimeout)
    }

    fn ensure_dhcp(&mut self) -> Result<NetworkConfig, NetError> {
        if let Some(config) = self.network_config {
            return Ok(config);
        }
        let mut phy = NagiPhyDevice::new(&mut self.device);
        let mut interface = new_interface(&mut phy)?;
        let mut storage: [SocketStorage<'_>; 8] = array::from_fn(|_| SocketStorage::EMPTY);
        let mut sockets = SocketSet::new(&mut storage[..]);
        let dhcp_handle = sockets.add(dhcpv4::Socket::new());
        let config = configure_dhcp(&mut interface, &mut phy, &mut sockets, dhcp_handle)?;
        self.network_config = Some(config);
        Ok(config)
    }
}

/// Bounded user-space socket-facing API for Nagi services and PAL adapters.
///
/// This facade deliberately exposes protocol operations without exposing
/// smoltcp internals or the raw VirtIO capability. Callers receive only the
/// attenuated result of the `Device` they supplied.
pub struct SocketApi<D> {
    stack: SmoltcpStack<D>,
}

impl<D: Device> SocketApi<D> {
    pub const fn new(device: D) -> Self {
        Self {
            stack: SmoltcpStack::new(device),
        }
    }

    pub fn device_mut(&mut self) -> &mut D {
        self.stack.device_mut()
    }

    pub fn dhcp(&mut self) -> Result<Ipv4Address, NetError> {
        self.stack.dhcp()
    }

    pub fn dhcp_gateway(&mut self) -> Result<Ipv4Address, NetError> {
        self.stack.dhcp_gateway()
    }

    pub fn resolve_ipv4(&mut self, name: &str) -> Result<Ipv4Address, NetError> {
        self.stack.resolve_ipv4(name)
    }

    pub fn icmp_echo(&mut self, target: Ipv4Address) -> Result<(), NetError> {
        self.stack.icmp_echo(target)
    }

    pub fn http_get(
        &mut self,
        target: Ipv4Address,
        target_port: u16,
        path: &[u8],
        expected_body: &[u8],
        response: &mut [u8],
    ) -> Result<usize, NetError> {
        self.stack
            .http_get(target, target_port, path, expected_body, response)
    }

    pub fn tcp_connect(
        &mut self,
        target: Ipv4Address,
        target_port: u16,
    ) -> Result<TcpConnectionId, NetError> {
        self.stack.tcp_connect(target, target_port)
    }

    pub fn tcp_send(
        &mut self,
        connection: TcpConnectionId,
        data: &[u8],
    ) -> Result<usize, NetError> {
        self.stack.tcp_send(connection, data)
    }

    pub fn tcp_try_send(
        &mut self,
        connection: TcpConnectionId,
        data: &[u8],
    ) -> Result<usize, NetError> {
        self.stack.tcp_try_send(connection, data)
    }

    pub fn tcp_shutdown_write(&mut self, connection: TcpConnectionId) -> Result<(), NetError> {
        self.stack.tcp_shutdown_write(connection)
    }

    pub fn tcp_set_nagle(
        &mut self,
        connection: TcpConnectionId,
        enabled: bool,
    ) -> Result<(), NetError> {
        self.stack.tcp_set_nagle(connection, enabled)
    }

    pub fn tcp_set_timeout(
        &mut self,
        connection: TcpConnectionId,
        timeout: Option<core::time::Duration>,
    ) -> Result<(), NetError> {
        self.stack.tcp_set_timeout(
            connection,
            timeout.map(|duration| (duration.as_secs(), duration.subsec_micros())),
        )
    }

    pub fn tcp_receive(
        &mut self,
        connection: TcpConnectionId,
        buffer: &mut [u8],
    ) -> Result<usize, NetError> {
        self.stack.tcp_receive(connection, buffer)
    }

    pub fn tcp_try_receive(
        &mut self,
        connection: TcpConnectionId,
        buffer: &mut [u8],
    ) -> Result<usize, NetError> {
        self.stack.tcp_try_receive(connection, buffer)
    }

    pub fn tcp_ready(
        &mut self,
        connection: TcpConnectionId,
        requested: i16,
    ) -> Result<i16, NetError> {
        self.stack.tcp_ready(connection, requested)
    }

    pub fn tcp_close(&mut self, connection: TcpConnectionId) -> Result<(), NetError> {
        self.stack.tcp_close(connection)
    }

    pub fn tcp_local_name(
        &self,
        connection: TcpConnectionId,
    ) -> Result<(Ipv4Address, u16), NetError> {
        self.stack.tcp_local_name(connection)
    }
}

#[cfg(all(feature = "m18-browser-network", target_os = "nagi"))]
fn interface_random_seed() -> Result<u64, NetError> {
    let mut seed = [0_u8; 8];
    libnagi::random_fill(&mut seed)
        .then(|| u64::from_le_bytes(seed))
        .ok_or(NetError::EntropyUnavailable)
}

#[cfg(not(all(feature = "m18-browser-network", target_os = "nagi")))]
fn interface_random_seed() -> Result<u64, NetError> {
    Ok(0x4e41_4749_0000_0001)
}

fn first_free_tcp_slot<T>(slots: &[Option<T>]) -> Option<usize> {
    slots.iter().position(Option::is_none)
}

fn new_interface<D: Device>(phy: &mut NagiPhyDevice<'_, D>) -> Result<Interface, NetError> {
    let mut config = InterfaceConfig::new(HardwareAddress::Ethernet(EthernetAddress::from_bytes(
        &LOCAL_MAC,
    )));
    // M18 rebuilds the smoltcp interface for each bounded network operation.
    // Its PRNG generates DNS transaction IDs and UDP/TCP source ports, so each
    // instance must start from fresh guest entropy to avoid repeating flows.
    config.random_seed = interface_random_seed()?;
    Ok(Interface::new(config, phy, now()))
}

/// Run one bounded network-service step.
///
/// `Interface::poll` drains the entire device RX queue and is intentionally
/// unbounded in smoltcp. Nagi services must preserve their own scheduling and
/// syscall budgets, so ingress is limited to one frame before the bounded
/// egress pass.
fn poll_once<D: Device>(
    interface: &mut Interface,
    phy: &mut NagiPhyDevice<'_, D>,
    sockets: &mut SocketSet<'_>,
    timestamp: Instant,
) {
    let _ = interface.poll_ingress_single(timestamp, phy, sockets);
    let _ = interface.poll_egress(timestamp, phy, sockets);
}

#[cfg(not(feature = "m18-browser-network"))]
fn reset_tcp_storage() {
    unsafe {
        let sockets = core::ptr::addr_of_mut!(TCP_SOCKET_STORAGE).cast::<SocketStorage<'static>>();
        for index in 0..TCP_CONNECTION_CAPACITY {
            core::ptr::write(sockets.add(index), SocketStorage::EMPTY);
        }
        core::ptr::write_bytes(
            core::ptr::addr_of_mut!(TCP_RX_STORAGE).cast::<u8>(),
            0,
            core::mem::size_of::<[[u8; SOCKET_BUFFER_SIZE]; TCP_CONNECTION_CAPACITY]>(),
        );
        core::ptr::write_bytes(
            core::ptr::addr_of_mut!(TCP_TX_STORAGE).cast::<u8>(),
            0,
            core::mem::size_of::<[[u8; SOCKET_BUFFER_SIZE]; TCP_CONNECTION_CAPACITY]>(),
        );
    }
}

fn apply_network_config(interface: &mut Interface, config: NetworkConfig) -> Result<(), NetError> {
    interface.update_ip_addrs(|addrs| {
        addrs.clear();
        let _ = addrs.push(IpCidr::Ipv4(config.address));
    });
    if let Some(router) = config.router {
        interface
            .routes_mut()
            .add_default_ipv4_route(router)
            .map_err(|_| NetError::RouteTableFull)?;
    }
    Ok(())
}

fn configure_dhcp<D: Device>(
    interface: &mut Interface,
    phy: &mut NagiPhyDevice<'_, D>,
    sockets: &mut SocketSet<'_>,
    handle: SocketHandle,
) -> Result<NetworkConfig, NetError> {
    let started = now();
    for _ in 0..POLL_BUDGET {
        let timestamp = now();
        let _ = interface.poll_ingress_single(timestamp, phy, sockets);
        let _ = interface.poll_egress(timestamp, phy, sockets);
        let event = sockets.get_mut::<dhcpv4::Socket>(handle).poll();
        if let Some(dhcpv4::Event::Configured(config)) = event {
            let network_config = NetworkConfig {
                address: config.address,
                router: config.router,
                dns: config.dns_servers.first().copied(),
            };
            apply_network_config(interface, network_config)?;
            return Ok(network_config);
        }
        if timestamp - started >= DHCP_TIMEOUT {
            break;
        }
    }
    Err(NetError::DhcpTimeout)
}

fn http_request(path: &[u8], request: &mut [u8; 512]) -> Result<usize, NetError> {
    let prefix = b"GET ";
    let suffix = b" HTTP/1.0\r\nHost: nagi\r\nConnection: close\r\n\r\n";
    let total = prefix
        .len()
        .checked_add(path.len())
        .and_then(|length| length.checked_add(suffix.len()))
        .ok_or(NetError::BufferTooSmall)?;
    if total > request.len() {
        return Err(NetError::BufferTooSmall);
    }
    request[..prefix.len()].copy_from_slice(prefix);
    let path_start = prefix.len();
    request[path_start..path_start + path.len()].copy_from_slice(path);
    let suffix_start = path_start + path.len();
    request[suffix_start..suffix_start + suffix.len()].copy_from_slice(suffix);
    Ok(total)
}

fn encode_icmp_echo_request(
    buffer: &mut [u8],
    ident: u16,
    seq_no: u16,
    data: &[u8],
) -> Result<usize, NetError> {
    let representation = Icmpv4Repr::EchoRequest {
        ident,
        seq_no,
        data,
    };
    let length = representation.buffer_len();
    if buffer.len() < length {
        return Err(NetError::BufferTooSmall);
    }
    let mut packet = Icmpv4Packet::new_unchecked(&mut buffer[..length]);
    representation.emit(&mut packet, &ChecksumCapabilities::default());
    Ok(length)
}

fn contains_http_ok(response: &[u8], length: usize, expected_body: &[u8]) -> bool {
    let bytes = &response[..length.min(response.len())];
    (bytes
        .windows(b"HTTP/1.0 200".len())
        .any(|window| window == b"HTTP/1.0 200")
        || bytes
            .windows(b"HTTP/1.1 200".len())
            .any(|window| window == b"HTTP/1.1 200"))
        && bytes
            .windows(expected_body.len())
            .any(|window| window == expected_body)
}

fn to_smol_ipv4(address: Ipv4Address) -> SmolIpv4 {
    SmolIpv4::new(address.0[0], address.0[1], address.0[2], address.0[3])
}

fn from_smol_ipv4(address: SmolIpv4) -> Ipv4Address {
    Ipv4Address::new(address.octets())
}

#[cfg(target_os = "nagi")]
fn now() -> Instant {
    let millis = libnagi::time_ticks()
        .saturating_mul(10)
        .min(i64::MAX as u64) as i64;
    Instant::from_millis(millis)
}

#[cfg(not(target_os = "nagi"))]
fn now() -> Instant {
    Instant::from_millis(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smoltcp::phy::ChecksumCapabilities;
    use smoltcp::wire::{Icmpv4Packet, Icmpv4Repr};

    struct RecordingDevice {
        sent: usize,
    }

    struct CountingIngressDevice {
        receives: usize,
    }

    // The production service is single-threaded and the test adapter uses
    // process-wide packet buffers to model that bounded guest service.
    static TEST_LOCK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

    struct TestLock;

    impl Drop for TestLock {
        fn drop(&mut self) {
            TEST_LOCK.store(false, core::sync::atomic::Ordering::Release);
        }
    }

    fn test_lock() -> TestLock {
        while TEST_LOCK
            .compare_exchange(
                false,
                true,
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
        {
            core::hint::spin_loop();
        }
        TestLock
    }

    impl Device for RecordingDevice {
        fn send(&mut self, frame: &[u8]) -> Result<(), NetError> {
            self.sent = self.sent.saturating_add(frame.len());
            Ok(())
        }

        fn receive(&mut self, _frame: &mut [u8; MAX_NET_FRAME_SIZE]) -> Result<usize, NetError> {
            Ok(0)
        }
    }

    impl Device for CountingIngressDevice {
        fn send(&mut self, _frame: &[u8]) -> Result<(), NetError> {
            Ok(())
        }

        fn receive(&mut self, frame: &mut [u8; MAX_NET_FRAME_SIZE]) -> Result<usize, NetError> {
            self.receives = self.receives.saturating_add(1);
            frame[..60].fill(0);
            Ok(60)
        }
    }

    #[test]
    fn bounded_poll_processes_at_most_one_ingress_frame() {
        let _lock = test_lock();
        let mut device = CountingIngressDevice { receives: 0 };
        let mut phy = NagiPhyDevice::new(&mut device);
        let mut interface = new_interface(&mut phy).expect("test network interface seed");
        let mut storage: [SocketStorage<'_>; 1] = array::from_fn(|_| SocketStorage::EMPTY);
        let mut sockets = SocketSet::new(&mut storage[..]);

        poll_once(&mut interface, &mut phy, &mut sockets, Instant::ZERO);

        assert_eq!(device.receives, 1);
    }

    #[test]
    fn dhcp_egress_is_bounded_and_uses_the_device_boundary() {
        let _lock = test_lock();
        let mut device = RecordingDevice { sent: 0 };
        let mut phy = NagiPhyDevice::new(&mut device);
        let mut interface = new_interface(&mut phy).expect("test network interface seed");
        let mut storage: [SocketStorage<'_>; 2] = array::from_fn(|_| SocketStorage::EMPTY);
        let mut sockets = SocketSet::new(&mut storage[..]);
        let handle = sockets.add(dhcpv4::Socket::new());
        let _ = interface.poll_egress(Instant::ZERO, &mut phy, &mut sockets);
        assert!(device.sent > 0);
        let _ = handle;
    }

    #[test]
    fn dns_egress_is_bounded_after_static_network_configuration() {
        let _lock = test_lock();
        let mut device = RecordingDevice { sent: 0 };
        let mut phy = NagiPhyDevice::new(&mut device);
        let mut interface = new_interface(&mut phy).expect("test network interface seed");
        let config = NetworkConfig {
            address: smoltcp::wire::Ipv4Cidr::new(SmolIpv4::new(10, 0, 2, 15), 24),
            router: Some(SmolIpv4::new(10, 0, 2, 2)),
            dns: Some(SmolIpv4::new(10, 0, 2, 3)),
        };
        apply_network_config(&mut interface, config).expect("network configuration");
        let mut storage: [SocketStorage<'_>; 2] = array::from_fn(|_| SocketStorage::EMPTY);
        let mut sockets = SocketSet::new(&mut storage[..]);
        let mut queries: [Option<dns::DnsQuery>; 1] = array::from_fn(|_| None);
        let handle = sockets.add(dns::Socket::new(
            &[IpAddress::Ipv4(SmolIpv4::new(10, 0, 2, 3))],
            &mut queries[..],
        ));
        let query = sockets
            .get_mut::<dns::Socket>(handle)
            .start_query(
                interface.context(),
                "example.com",
                smoltcp::wire::DnsQueryType::A,
            )
            .expect("DNS query");

        let _ = interface.poll_egress(Instant::ZERO, &mut phy, &mut sockets);

        assert!(device.sent > 0);
        assert!(matches!(
            sockets
                .get_mut::<dns::Socket>(handle)
                .get_query_result(query),
            Err(dns::GetQueryResultError::Pending)
        ));
    }

    #[test]
    fn dns_a_answers_distinguish_success_from_missing_records() {
        assert_eq!(
            dns_a_result(&[IpAddress::Ipv4(SmolIpv4::new(203, 0, 113, 18))]),
            Ok(Ipv4Address::new([203, 0, 113, 18]))
        );
        assert_eq!(dns_a_result(&[]), Err(NetError::DnsFailure));
    }

    #[test]
    fn icmp_echo_request_is_encoded_as_a_real_wire_packet() {
        let _lock = test_lock();
        let mut packet = [0_u8; 64];
        let length = encode_icmp_echo_request(&mut packet, 0x4e47, 1, b"NAGI-ICMP")
            .expect("ICMP packet fits");
        let parsed = Icmpv4Packet::new_checked(&packet[..length]).expect("ICMP packet");
        let representation = Icmpv4Repr::parse(&parsed, &ChecksumCapabilities::default())
            .expect("valid ICMP representation");
        assert_eq!(
            representation,
            Icmpv4Repr::EchoRequest {
                ident: 0x4e47,
                seq_no: 1,
                data: b"NAGI-ICMP",
            }
        );
    }

    #[test]
    fn retained_tcp_surface_fails_closed_before_connect() {
        let _lock = test_lock();
        let mut stack = SocketApi::new(RecordingDevice { sent: 0 });
        let connection = TcpConnectionId::from_index(0);
        assert_eq!(
            stack.tcp_send(connection, b"GET / HTTP/1.0\r\n\r\n"),
            Err(NetError::ConnectionReset)
        );
        assert_eq!(
            stack.tcp_try_send(connection, b"GET / HTTP/1.1\r\n\r\n"),
            Err(NetError::ConnectionReset)
        );
        let mut byte = [0];
        assert_eq!(
            stack.tcp_try_receive(connection, &mut byte),
            Err(NetError::ConnectionReset)
        );
        assert_eq!(
            stack.tcp_ready(connection, SOCKET_READY_READ),
            Err(NetError::ConnectionReset)
        );
        assert_eq!(stack.tcp_close(connection), Err(NetError::ConnectionReset));
    }

    #[test]
    fn tcp_connection_capacity_keeps_free_slots_available() {
        assert_eq!(first_free_tcp_slot(&[Some(()), None, None]), Some(1));
        assert_eq!(first_free_tcp_slot(&[Some(()), Some(())]), None);
    }
}
