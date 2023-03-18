//! RADIUS client implementation.

use std::net::SocketAddr;
use std::time::Duration;

use thiserror::Error;
use tokio::net::UdpSocket;
use tokio::time::timeout;

use crate::core::packet::Packet;

#[derive(Error, Debug)]
pub enum ClientError {
    /// This error is occurred when UDP socket binding has been failed.
    #[error("failed to bind a UDP socket; {0}")]
    FailedUdpSocketBindingError(String),

    /// This error is raised when it failed to establish the connection.
    #[error("failed to establish a UDP connection to {0}; {1}")]
    FailedEstablishingUdpConnectionError(String, String),

    /// This error is raised when encoding RADIUS packet has been failed.
    #[error("failed to encode a RADIUS request; {0}")]
    FailedRadiusPacketEncodingError(String),

    /// This error is raised when it fails to send a RADIUS packet.
    #[error("failed to send a UDP datagram to {0}; {1}")]
    FailedSendingRadiusPacketError(String, String),

    /// This error is raised when it fails to receive a RADIUS response.
    #[error("failed to receive the UDP response from {0}; {1}")]
    FailedReceivingResponseError(String, String),

    /// This error is raised when it fails to decode a RADIUS response packet.
    #[error("failed to decode a RADIUS response packet; {0}")]
    FailedDecodingRadiusResponseError(String),

    /// This error is raised when it exceeds the connection timeout duration.
    /// Connection timeout means it fails to establish a connection in time.
    #[error("connection timeout")]
    ConnectionTimeoutError(),

    /// This error is raised when it exceeds the socket timeout duration.
    /// Socket timeout means it fails to receive a response from the request target in time.
    #[error("socket timeout")]
    SocketTimeoutError(),
}

/// A basic implementation of the RADIUS client.
pub struct Client {
    connection_timeout: Option<Duration>,
    socket_timeout: Option<Duration>,
}

impl Client {
    const MAX_DATAGRAM_SIZE: usize = 65507;

    /// A constructor for a client.
    ///
    /// # Arguments
    ///
    /// * `connection_timeout` - A duration of connection timeout. If the connection is not established in time, the `ConnectionTimeoutError` occurs.
    ///                          If this value is `None`, it never timed-out.
    /// * `socket_timeout` - A duration of socket timeout. If the response is not returned in time, the `SocketTimeoutError` occurs.
    ///                      If this value is `None`, it never timed-out.
    pub fn new(connection_timeout: Option<Duration>, socket_timeout: Option<Duration>) -> Self {
        Client {
            connection_timeout,
            socket_timeout,
        }
    }

    /// This method sends a packet to the destination.
    ///
    /// This method doesn't support auto retransmission when something failed, so if you need such a feature you have to implement that.
    pub async fn send_packet(
        &self,
        remote_addr: &SocketAddr,
        request_packet: &Packet,
    ) -> Result<Packet, ClientError> {
        let local_addr: SocketAddr = if remote_addr.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        }
        .parse()
        .unwrap();

        let conn = match UdpSocket::bind(local_addr).await {
            Ok(conn) => conn,
            Err(e) => return Err(ClientError::FailedUdpSocketBindingError(e.to_string())),
        };

        match self.connection_timeout {
            Some(connection_timeout) => {
                match timeout(connection_timeout, self.connect(&conn, remote_addr)).await {
                    Ok(conn_establish_res) => conn_establish_res,
                    Err(_) => Err(ClientError::ConnectionTimeoutError()),
                }
            }
            None => self.connect(&conn, remote_addr).await,
        }?;

        let request_data = match request_packet.encode() {
            Ok(encoded) => encoded,
            Err(e) => return Err(ClientError::FailedRadiusPacketEncodingError(format!("{e}"))),
        };

        let response = match self.socket_timeout {
            Some(socket_timeout) => {
                match timeout(
                    socket_timeout,
                    self.request(&conn, &request_data, remote_addr),
                )
                .await
                {
                    Ok(response) => response,
                    Err(_) => Err(ClientError::SocketTimeoutError()),
                }
            }
            None => self.request(&conn, &request_data, remote_addr).await,
        }?;

        match Packet::decode(&response.to_vec(), request_packet.get_secret()) {
            Ok(response_packet) => Ok(response_packet),
            Err(e) => Err(ClientError::FailedDecodingRadiusResponseError(format!(
                "{e}"
            ))),
        }
    }

    async fn connect(&self, conn: &UdpSocket, remote_addr: &SocketAddr) -> Result<(), ClientError> {
        match conn.connect(remote_addr).await {
            Ok(_) => Ok(()),
            Err(e) => Err(ClientError::FailedEstablishingUdpConnectionError(
                remote_addr.to_string(),
                e.to_string(),
            )),
        }
    }

    async fn request(
        &self,
        conn: &UdpSocket,
        request_data: &[u8],
        remote_addr: &SocketAddr,
    ) -> Result<Vec<u8>, ClientError> {
        match conn.send(request_data).await {
            Ok(_) => {}
            Err(e) => {
                return Err(ClientError::FailedSendingRadiusPacketError(
                    remote_addr.to_string(),
                    e.to_string(),
                ))
            }
        };

        let mut buf = vec![0; Self::MAX_DATAGRAM_SIZE];
        match conn.recv(&mut buf).await {
            Ok(len) => Ok(buf[..len].to_vec()),
            Err(e) => Err(ClientError::FailedReceivingResponseError(
                remote_addr.to_string(),
                e.to_string(),
            )),
        }
    }
}
