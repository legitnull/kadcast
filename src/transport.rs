// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
//
// Copyright (c) DUSK NETWORK. All rights reserved.

use std::{error::Error, net::SocketAddr};

use tokio::{
    io,
    net::UdpSocket,
    sync::mpsc::{Receiver, Sender},
};
use tracing::*;

use crate::{
    encoding::{message::Message, Marshallable},
    peer::PeerNode,
    transport::encoding::{Encoder, RaptorQEncoder},
    MAX_DATAGRAM_SIZE,
};

pub(crate) type MessageBeanOut = (Message, Vec<SocketAddr>);
pub(crate) type MessageBeanIn = (Message, SocketAddr);

pub(crate) struct WireNetwork {}

mod encoding;

impl WireNetwork {
    pub async fn start(
        inbound_channel_tx: Sender<MessageBeanIn>,
        public_ip: String,
        outbound_channel_rx: Receiver<MessageBeanOut>,
    ) {
        let public_address = public_ip
            .parse()
            .expect("Unable to parse public_ip address");
        let a = WireNetwork::listen_out(outbound_channel_rx);
        let b =
            WireNetwork::listen_in(public_address, inbound_channel_tx.clone());
        let _ = tokio::join!(a, b);
    }

    async fn listen_in(
        public_address: SocketAddr,
        inbound_channel_tx: Sender<MessageBeanIn>,
    ) -> io::Result<()> {
        debug!("WireNetwork::listen_in started");
        let mut decoder = RaptorQEncoder::new();
        let socket = UdpSocket::bind(public_address)
            .await
            .expect("Unable to bind address");
        info!("Listening on: {}", socket.local_addr()?);
        loop {
            let mut bytes = [0; MAX_DATAGRAM_SIZE];
            let (_, addr) = socket.recv_from(&mut bytes).await?;

            match Message::unmarshal_binary(&mut &bytes[..]) {
                Ok(deser) => {
                    trace!("> Received {:?}", deser);
                    let to_process = decoder.decode(deser);
                    if let Some(message) = to_process {
                        let valid_header = PeerNode::verify_header(
                            message.header(),
                            &addr.ip(),
                        );
                        match valid_header {
                            true => {
                                //FIX_ME: use send.await instead of try_send
                                let _ = inbound_channel_tx
                                    .try_send((message, addr));
                            }
                            false => {
                                error!(
                                    "Invalid Id {:?} - {}",
                                    message.header(),
                                    &addr.ip()
                                );
                            }
                        }
                    }
                }
                Err(e) => error!("Error deser from {} - {}", addr, e),
            }
        }
    }

    async fn listen_out(
        mut outbound_channel_rx: Receiver<MessageBeanOut>,
    ) -> io::Result<()> {
        debug!("WireNetwork::listen_out started");
        loop {
            if let Some((message, to)) = outbound_channel_rx.recv().await {
                trace!("< Message to send to ({:?}) - {:?} ", to, message);
                for chunk in RaptorQEncoder::encode(message).iter() {
                    let bytes = chunk.bytes();
                    for remote_addr in to.iter() {
                        let _ = WireNetwork::send(&bytes, remote_addr)
                            .await
                            .map_err(|e| warn!("Unable to send msg {}", e));
                    }
                }
            }
        }
    }

    async fn send(
        data: &[u8],
        remote_addr: &SocketAddr,
    ) -> Result<(), Box<dyn Error>> {
        let local_addr: SocketAddr = if remote_addr.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        }
        .parse()?;
        let socket = UdpSocket::bind(local_addr).await?;
        // const MAX_DATAGRAM_SIZE: usize = 65_507;
        socket.connect(&remote_addr).await?;
        socket.send(data).await?;
        Ok(())
    }
}
