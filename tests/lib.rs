// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
//
// Copyright (c) DUSK NETWORK. All rights reserved.

#[cfg(test)]
mod tests {

    use std::{
        collections::HashMap,
        net::{SocketAddr, ToSocketAddrs},
        time::Duration,
    };

    use kadcast::{
        MessageInfo, NetworkListen, Peer,
        BUCKET_DEFAULT_NODE_EVICT_AFTER_MILLIS, BUCKET_DEFAULT_NODE_TTL_MILLIS,
        BUCKET_DEFAULT_TTL_SECS,
    };
    use tokio::{sync::mpsc, time::timeout};
    use tracing::info;
    use tracing::warn;

    #[test]
    fn resolvetest() {
        let server_details = "192.168.1.5:80";
        let server: Vec<_> = server_details
            .to_socket_addrs()
            .expect("Unable to resolve domain")
            .collect();
        println!("{:?}", server);
    }

    const NODES: i32 = 20;
    const BASE_PORT: i32 = 20000;
    const BOOTSTRAP_COUNT: i32 = 2;
    const WAIT_SEC: u64 = 20;
    const MESSAGE_SIZE: usize = 100_000;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn harness_test() {
        // Generate a subscriber with the desired log level.
        let subscriber = tracing_subscriber::fmt::Subscriber::builder()
            .with_max_level(tracing::Level::INFO)
            .finish();
        // Set the subscriber as global.
        // so this subscriber will be used as the default in all threads for the
        // remainder of the duration of the program, similar to how `loggers`
        // work in the `log` crate.
        tracing::subscriber::set_global_default(subscriber)
            .expect("Failed on subscribe tracing");
        let (tx, rx) = mpsc::channel(100);

        let bootstraps = {
            let mut v = vec![];
            for i in 0..BOOTSTRAP_COUNT {
                v.push(format!("127.0.0.1:{}", BASE_PORT + i).to_string());
            }
            v
        };
        let mut peers = HashMap::new();
        let peer0 = create_peer(0, bootstraps.clone(), tx.clone());
        peers.insert(0, peer0);
        // peers.pu;
        for i in 1..NODES {
            tokio::time::sleep(Duration::from_millis(500)).await;
            peers.insert(i, create_peer(i, bootstraps.clone(), tx.clone()));
        }
        tokio::time::sleep(Duration::from_millis(2000)).await;
        let mut data: Vec<u8> = vec![0; MESSAGE_SIZE];
        for i in 0..data.len() {
            data[i] = rand::Rng::gen(&mut rand::thread_rng());
        }
        for i in 1..NODES {
            // for (i, p) in peers.iter() {
            info!("ROUTING TABLE PEER #{}", i);
            peers.get(&i).unwrap().report().await;
            info!("----------------------");
        }

        peers.get(&0).unwrap().broadcast(&data, None).await;
        let res =
            timeout(Duration::from_secs(WAIT_SEC), receive(rx, NODES - 1))
                .await;
        assert!(
            res.is_ok(),
            "Not all nodes received the broadcasted message"
        );
    }

    async fn receive(
        mut rx: mpsc::Receiver<(usize, (Vec<u8>, SocketAddr, u8))>,
        expected: i32,
    ) {
        let mut missing = HashMap::new();
        for i in (BASE_PORT + 1)..(BASE_PORT + expected) {
            missing.insert(i, i);
        }
        let mut i = 0;
        while !missing.is_empty() {
            if let Some((receiver_port, message)) = rx.recv().await {
                i = i + 1;
                missing.remove(&(receiver_port as i32));
                info!(
                    "RECEIVER PORT: {} - Message N° {} got from {:?}",
                    receiver_port, i, message.1
                );
            }
        }
        info!("Received All {} messages", i);
    }

    fn create_peer(
        i: i32,
        bootstrap: Vec<String>,
        grpc_sender: mpsc::Sender<(usize, (Vec<u8>, SocketAddr, u8))>,
    ) -> Peer {
        let port = BASE_PORT + i;
        let public_addr = format!("127.0.0.1:{}", port).to_string();
        let listener = KadcastListener {
            grpc_sender,
            receiver_port: port as usize,
        };
        let mut peer_builder = Peer::builder(public_addr, bootstrap, listener)
            // .with_listen_address(listen_addr)
            .with_node_ttl(Duration::from_millis(
                BUCKET_DEFAULT_NODE_TTL_MILLIS,
            ))
            .with_bucket_ttl(Duration::from_secs(BUCKET_DEFAULT_TTL_SECS))
            .with_channel_size(100)
            .with_node_evict_after(Duration::from_millis(
                BUCKET_DEFAULT_NODE_EVICT_AFTER_MILLIS,
            ))
            .with_auto_propagate(true);

        // Disable recursive discovery in a local env
        // This should be set to `true` in a real env
        peer_builder = peer_builder.with_recursive_discovery(true);

        //this is unusefull, just to get the default conf
        peer_builder
            .transport_conf()
            .extend(kadcast::transport::default_configuration());

        //RaptorQ Decoder conf
        peer_builder
            .transport_conf()
            .insert("cache_ttl_secs".to_string(), "60".to_string());
        peer_builder
            .transport_conf()
            .insert("cache_prune_every_secs".to_string(), "300".to_string());

        //RaptorQ Encoder conf
        peer_builder.transport_conf().insert(
            "min_repair_packets_per_block".to_string(),
            "5".to_string(),
        );
        peer_builder
            .transport_conf()
            .insert("mtu".to_string(), "1300".to_string());
        peer_builder
            .transport_conf()
            .insert("fec_redundancy".to_string(), "0.15".to_string());

        // //UDP conf

        peer_builder
            .transport_conf()
            .insert("udp_backoff_timeout_micros".to_string(), "0".to_string());
        peer_builder
            .transport_conf()
            .insert("udp_recv_buffer_size".to_string(), "SYSTEM".to_string());
        peer_builder
            .transport_conf()
            .insert("udp_send_retry_count".to_string(), "3".to_string());
        peer_builder.transport_conf().insert(
            "udp_send_retry_interval_millis".to_string(),
            "5".to_string(),
        );

        peer_builder.build()
    }

    struct KadcastListener {
        grpc_sender: mpsc::Sender<(usize, (Vec<u8>, SocketAddr, u8))>,
        receiver_port: usize,
    }

    impl NetworkListen for KadcastListener {
        fn on_message(&self, message: Vec<u8>, metadata: MessageInfo) {
            self.grpc_sender
                .try_send((
                    self.receiver_port,
                    (message, metadata.src(), metadata.height()),
                ))
                .unwrap_or_else(|e| {
                    warn!("Error sending to listener {}", e);
                });
        }
    }
}
