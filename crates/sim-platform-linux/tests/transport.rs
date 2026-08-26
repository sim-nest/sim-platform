use sim_kernel::{Cx, DefaultFactory, EagerPolicy, HandleSeed};
use sim_lib_server::{ServerAddress, ServerTransport, TcpServerTransport};
use sim_platform_linux::{LinuxDnsPort, LinuxIpcPort, LinuxSocketPort, bind_transport_services};
use sim_transport_ports::{
    DnsPort, Half, IpcAddress, IpcPort, SocketAddress, SocketPort, TransportErrorKind,
};
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

#[test]
fn server_tcp_transport_uses_the_explicit_linux_capsule() {
    bind_transport_services().unwrap();
    let listener = TcpServerTransport::bind(ServerAddress::Tcp {
        host: "127.0.0.1".to_owned(),
        port: 0,
    })
    .unwrap();
    let ServerAddress::Tcp { host, port } = listener.address() else {
        panic!("expected the bound TCP address");
    };
    let client = LinuxSocketPort
        .connect_tcp(&SocketAddress::Ip {
            address: host.parse().unwrap(),
            port: *port,
        })
        .unwrap();
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(0x5a4f_50a1_71a0_0001),
    );
    let _server = loop {
        if let Some(connection) = listener
            .accept_timeout(&mut cx, Duration::from_millis(25))
            .unwrap()
        {
            break connection;
        }
    };
    client.shutdown(Half::Both).unwrap();
    listener.shutdown(&mut cx).unwrap();
}

#[test]
fn tcp_udp_dns_and_native_errors_are_capsule_owned() {
    let port = LinuxSocketPort;
    let loopback = SocketAddress::Ip {
        address: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: 0,
    };
    let listener = port.listen_tcp(&loopback).unwrap();
    let bound = listener.local_address().unwrap();
    let mut client = port.connect_tcp(&bound).unwrap();
    let mut server = loop {
        if let Some(stream) = listener.accept().unwrap() {
            break stream;
        }
    };
    client.write_all(b"same-frame-bytes").unwrap();
    let mut capture = [0; 16];
    server.read_exact(&mut capture).unwrap();
    assert_eq!(&capture, b"same-frame-bytes");

    let udp = port.bind_udp(&loopback).unwrap();
    assert_ne!(udp.local_address().unwrap(), loopback);
    assert!(!LinuxDnsPort.resolve("localhost", 9).unwrap().is_empty());

    let collision = port.listen_tcp(&bound).err().unwrap();
    assert_eq!(collision.kind, TransportErrorKind::AddressInUse);
}

#[test]
fn ipc_variants_do_not_share_a_path_contract() {
    let error = LinuxIpcPort
        .connect(&IpcAddress::WindowsPipe("sim-test".into()))
        .err()
        .unwrap();
    assert_eq!(error.kind, TransportErrorKind::Unsupported);

    #[cfg(unix)]
    {
        let path = std::env::temp_dir().join(format!("sim-platform-ipc-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = LinuxIpcPort
            .listen(&IpcAddress::UnixPath(path.clone()))
            .unwrap();
        let mut client = LinuxIpcPort
            .connect(&IpcAddress::UnixPath(path.clone()))
            .unwrap();
        let mut server = loop {
            if let Some(stream) = listener.accept().unwrap() {
                break stream;
            }
        };
        client.write_all(b"ipc").unwrap();
        let mut bytes = [0; 3];
        server.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"ipc");
        listener.close().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
