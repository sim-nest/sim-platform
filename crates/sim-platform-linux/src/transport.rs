use sim_transport_ports::{
    Datagram, DnsPort, Half, IpcAddress, IpcListener, IpcPort, Listener, Result, SocketAddress,
    SocketPort, Stream, TransportError, TransportErrorKind,
};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs, UdpSocket},
    time::Duration,
};

fn native(error: std::io::Error) -> TransportError {
    let kind = match error.kind() {
        std::io::ErrorKind::AddrInUse => TransportErrorKind::AddressInUse,
        std::io::ErrorKind::ConnectionRefused => TransportErrorKind::ConnectionRefused,
        std::io::ErrorKind::TimedOut => TransportErrorKind::TimedOut,
        std::io::ErrorKind::WouldBlock => TransportErrorKind::WouldBlock,
        std::io::ErrorKind::NotFound => TransportErrorKind::NotFound,
        std::io::ErrorKind::Interrupted => TransportErrorKind::Cancelled,
        _ => TransportErrorKind::ProviderFault,
    };
    TransportError::new(kind, error.to_string())
}
fn socket(address: &SocketAddress) -> std::net::SocketAddr {
    match address {
        SocketAddress::Ip { address, port } => (*address, *port).into(),
    }
}
fn wrapped(address: std::net::SocketAddr) -> SocketAddress {
    SocketAddress::Ip {
        address: address.ip(),
        port: address.port(),
    }
}

pub struct LinuxSocketPort;
impl SocketPort for LinuxSocketPort {
    fn listen_tcp(&self, address: &SocketAddress) -> Result<Box<dyn Listener>> {
        let listener = TcpListener::bind(socket(address)).map_err(native)?;
        listener.set_nonblocking(true).map_err(native)?;
        Ok(Box::new(NativeListener(listener)))
    }
    fn connect_tcp(&self, address: &SocketAddress) -> Result<Box<dyn Stream>> {
        let stream = TcpStream::connect(socket(address)).map_err(native)?;
        stream.set_nodelay(true).map_err(native)?;
        Ok(Box::new(NativeStream(stream)))
    }
    fn bind_udp(&self, address: &SocketAddress) -> Result<Box<dyn Datagram>> {
        let socket = UdpSocket::bind(socket(address)).map_err(native)?;
        socket.set_nonblocking(true).map_err(native)?;
        Ok(Box::new(NativeDatagram(socket)))
    }
}
struct NativeListener(TcpListener);
impl Listener for NativeListener {
    fn local_address(&self) -> Result<SocketAddress> {
        self.0.local_addr().map(wrapped).map_err(native)
    }
    fn accept(&self) -> Result<Option<Box<dyn Stream>>> {
        match self.0.accept() {
            Ok((s, _)) => {
                s.set_nodelay(true).map_err(native)?;
                Ok(Some(Box::new(NativeStream(s))))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(native(e)),
        }
    }
    fn close(&self) -> Result<()> {
        Ok(())
    }
}
struct NativeStream(TcpStream);
impl Read for NativeStream {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(b)
    }
}
impl Write for NativeStream {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl Stream for NativeStream {
    fn set_read_timeout(&self, t: Option<Duration>) -> Result<()> {
        self.0.set_read_timeout(t).map_err(native)
    }
    fn shutdown(&self, h: Half) -> Result<()> {
        self.0
            .shutdown(match h {
                Half::Read => Shutdown::Read,
                Half::Write => Shutdown::Write,
                Half::Both => Shutdown::Both,
            })
            .map_err(native)
    }
}
struct NativeDatagram(UdpSocket);
impl Datagram for NativeDatagram {
    fn local_address(&self) -> Result<SocketAddress> {
        self.0.local_addr().map(wrapped).map_err(native)
    }
    fn send_to(&mut self, b: &[u8], a: &SocketAddress) -> Result<usize> {
        self.0.send_to(b, socket(a)).map_err(native)
    }
    fn recv_from(&mut self, b: &mut [u8]) -> Result<Option<(usize, SocketAddress)>> {
        match self.0.recv_from(b) {
            Ok((n, a)) => Ok(Some((n, wrapped(a)))),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(native(e)),
        }
    }
    fn close(&self) -> Result<()> {
        Ok(())
    }
}
pub struct LinuxDnsPort;
impl DnsPort for LinuxDnsPort {
    fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddress>> {
        (host, port)
            .to_socket_addrs()
            .map(|v| v.map(wrapped).collect())
            .map_err(|e| TransportError::new(TransportErrorKind::DnsFailure, e.to_string()))
    }
}

/// Linux local IPC supports Unix paths only; Windows pipe names fail closed.
pub struct LinuxIpcPort;

/// Explicitly bind the Linux capsule's native transport realization.
pub fn bind_transport_services() -> Result<()> {
    sim_transport_ports::bind_services(sim_transport_ports::TransportServices {
        sockets: std::sync::Arc::new(LinuxSocketPort),
        dns: std::sync::Arc::new(LinuxDnsPort),
        ipc: Some(std::sync::Arc::new(LinuxIpcPort)),
    })
}
#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::net::{UnixListener, UnixStream};
    pub(super) struct UListener(pub UnixListener);
    pub(super) struct UStream(pub UnixStream);
    impl IpcListener for UListener {
        fn accept(&self) -> Result<Option<Box<dyn Stream>>> {
            match self.0.accept() {
                Ok((s, _)) => Ok(Some(Box::new(UStream(s)))),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
                Err(e) => Err(native(e)),
            }
        }
        fn close(&self) -> Result<()> {
            Ok(())
        }
    }
    impl Read for UStream {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(b)
        }
    }
    impl Write for UStream {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.write(b)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
    impl Stream for UStream {
        fn set_read_timeout(&self, t: Option<Duration>) -> Result<()> {
            self.0.set_read_timeout(t).map_err(native)
        }
        fn shutdown(&self, h: Half) -> Result<()> {
            self.0
                .shutdown(match h {
                    Half::Read => Shutdown::Read,
                    Half::Write => Shutdown::Write,
                    Half::Both => Shutdown::Both,
                })
                .map_err(native)
        }
    }
}
impl IpcPort for LinuxIpcPort {
    fn listen(&self, address: &IpcAddress) -> Result<Box<dyn IpcListener>> {
        match address {
            #[cfg(unix)]
            IpcAddress::UnixPath(path) => {
                let l = std::os::unix::net::UnixListener::bind(path).map_err(native)?;
                l.set_nonblocking(true).map_err(native)?;
                Ok(Box::new(unix::UListener(l)))
            }
            _ => Err(TransportError::new(
                TransportErrorKind::Unsupported,
                "Linux IPC requires a UnixPath",
            )),
        }
    }
    fn connect(&self, address: &IpcAddress) -> Result<Box<dyn Stream>> {
        match address {
            #[cfg(unix)]
            IpcAddress::UnixPath(path) => std::os::unix::net::UnixStream::connect(path)
                .map(|s| Box::new(unix::UStream(s)) as Box<dyn Stream>)
                .map_err(native),
            _ => Err(TransportError::new(
                TransportErrorKind::Unsupported,
                "Linux IPC requires a UnixPath",
            )),
        }
    }
}
