use std::io::{self, Read, Write};

use tokio_io::{AsyncRead, AsyncWrite};
use futures::Poll;
use client::network::stream::NetworkStream;
use std::net::SocketAddr;

#[cfg(feature = "rustls")]
pub mod stream {
    use tokio::net::TcpStream;
    use tokio_rustls::{TlsStream, rustls::ClientSession};
    use tokio_rustls::{rustls::internal::pemfile, rustls::ClientConfig, TlsConnector};
    use std::fs::File;
    use std::io::BufReader;
    use std::path::Path;
    use std::sync::Arc;
    use futures::{Future, future::Either};
    use error::ConnectError;
    use tokio_codec::{Decoder, Framed};
    use codec::MqttCodec;
    use std::path::PathBuf;
    use futures::future;
    use webpki::DNSNameRef;
    use client::network::lookup_ipv4;

    pub enum NetworkStream {
        Tcp(TcpStream),
        Tls(TlsStream<TcpStream, ClientSession>)
    }

    impl NetworkStream {
        pub fn builder() -> NetworkStreamBuilder {
            NetworkStreamBuilder {
                certificate_authority: None,
                client_cert: None,
                client_private_key: None,
            }
        }
    }

    pub struct NetworkStreamBuilder {
        certificate_authority: Option<PathBuf>,
        client_cert: Option<PathBuf>,
        client_private_key: Option<PathBuf>
    }

    impl NetworkStreamBuilder {
        pub fn add_certificate_authority<P: AsRef<Path>>(mut self, ca: P) -> NetworkStreamBuilder {
            let ca = ca.as_ref().to_path_buf();
            self.certificate_authority = Some(ca);
            self
        }

        pub fn add_client_auth<P: AsRef<Path>>(mut self, cert: P, private_key: P) -> NetworkStreamBuilder {
            let cert = cert.as_ref().to_path_buf();
            let private_key = private_key.as_ref().to_path_buf();

            self.client_cert = Some(cert);
            self.client_private_key = Some(private_key);
            self
        }

        fn create_stream(&mut self) -> Result<TlsConnector, ConnectError>{
            let mut config = ClientConfig::new();

            match self.certificate_authority.clone() {
                Some(certificate_authority) => {
                    let mut ca = BufReader::new(File::open(certificate_authority)?);
                    config.root_store.add_pem_file(&mut ca).unwrap();
                }
                None => return Err(ConnectError::NoCertificateAuthority)
            }


            match (self.client_cert.clone(), self.client_private_key.clone()) {
                (Some(cert), Some(key)) => {
                    let mut cert = BufReader::new(File::open(cert)?);
                    let mut keys = BufReader::new(File::open(key)?);

                    let certs = pemfile::certs(&mut cert).unwrap();
                    let keys =  pemfile::rsa_private_keys(&mut keys).unwrap();

                    config.set_single_client_cert(certs, keys[0].clone());
                }
                (None , None) => (),
                _ => unimplemented!()
            };

            Ok(TlsConnector::from(Arc::new(config)))
        }

        pub fn connect(mut self, host: &str, port: u16) -> impl Future<Item = Framed<NetworkStream, MqttCodec>, Error = ConnectError> {

            // let host = host.to_owned();
            let domain = DNSNameRef::try_from_ascii_str(host).unwrap().to_owned();
            let addr = lookup_ipv4(host, port);

            let tls_connector = self.create_stream();

            let network_future = match tls_connector {
                Ok(tls_connector) => {
                    let tls = TcpStream::connect(&addr)
                        .and_then(move |stream| tls_connector.connect(domain.as_ref(), stream))
                        .map_err(ConnectError::from)
                        .and_then(|stream| {
                            let stream = NetworkStream::Tls(stream);
                            future::ok(MqttCodec.framed(stream))
                        });

                    Either::A(tls)
                }
                Err(ConnectError::NoCertificateAuthority) => {
                    let tcp = TcpStream::connect(&addr)
                        .and_then(|stream| {
                            let stream = NetworkStream::Tcp(stream);
                            future::ok(MqttCodec.framed(stream))
                        })
                        .map_err(ConnectError::from);

                    Either::B(tcp)
                }
                _ => unimplemented!()
            };

            network_future
        }
    }
}


#[cfg(feature = "nativetls")]
mod stream {
    use tokio::net::TcpStream;
    use tokio_tls::TlsStream;

    pub enum NetworkStream {
        Tcp(TcpStream),
        Tls(TlsStream<TcpStream>)
    }

    impl NetworkStream {

    }
}


fn lookup_ipv4(host: &str, port: u16) -> SocketAddr {
    use std::net::ToSocketAddrs;

    let addrs = (host, port).to_socket_addrs().unwrap();
    for addr in addrs {
        if let SocketAddr::V4(_) = addr {
            return addr;
        }
    }

    unreachable!("Cannot lookup address");
}

impl Read for NetworkStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.read(buf),
            NetworkStream::Tls(ref mut s) => s.read(buf),
        }
    }
}

impl Write for NetworkStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.write(buf),
            NetworkStream::Tls(ref mut s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.flush(),
            NetworkStream::Tls(ref mut s) => s.flush(),
        }
    }
}

impl AsyncRead for NetworkStream{}
impl AsyncWrite for NetworkStream{
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        match *self {
            NetworkStream::Tcp(ref mut s) => s.shutdown(),
            NetworkStream::Tls(ref mut s) => s.shutdown(),
        }
    }
}