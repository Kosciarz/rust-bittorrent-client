use std::{
    io::{self, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream},
    time::Duration,
};

#[derive(Debug)]
pub struct Peer {
    socket: SocketAddr,
    chocked: bool,
    interested: bool,
}

impl Peer {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            socket: SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])),
                u16::from_be_bytes([bytes[4], bytes[5]]),
            ),
            chocked: true,
            interested: false,
        }
    }

    pub fn chocked(&self) -> bool {
        self.chocked
    }

    pub fn interested(&self) -> bool {
        self.interested
    }

    pub fn connect(&mut self) -> io::Result<()> {
        let mut connection = PeerConnection::new(&self.socket)?;
        println!("Connected to peer: {}", self.socket);

        Ok(())
    }
}

#[derive(Debug)]
struct PeerConnection {
    stream: TcpStream,
    handshake_sent: bool,
}

impl PeerConnection {
    fn new(socket: &SocketAddr) -> io::Result<Self> {
        Ok(Self {
            stream: TcpStream::connect_timeout(socket, Duration::from_secs(5))?,
            handshake_sent: false,
        })
    }
}
