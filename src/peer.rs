use crate::kbucket::{BinaryKey, BinaryNonce};
use crate::utils;
use blake2::{Blake2s, Digest};
use std::convert::TryInto;
use std::net::{IpAddr, SocketAddr};
pub type PeerNode = Node<PeerID, PeerInfo>;

use crate::kbucket::{BinaryID, Node};
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerInfo {
    address: SocketAddr,
}
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerID {
    str_id: String,
    binary: BinaryKey,
    nonce: BinaryNonce,
}

impl BinaryID for PeerID {
    fn as_binary(&self) -> &BinaryKey {
        &self.binary
    }

    fn nonce(&self) -> &BinaryNonce {
        &self.nonce
    }
}

impl PeerNode {
    pub fn from_address(address: String) -> PeerNode {
        let server: SocketAddr = address.parse().expect("Unable to parse address");
        let info = PeerInfo { address: server };
        let binary = PeerNode::compute_id(&info);
        let nonce = utils::compute_nonce(&binary);
        let id = PeerID {
            str_id: address,
            binary,
            nonce,
        };
        Node::new(id, info)
    }

    fn compute_id(info: &PeerInfo) -> BinaryKey {
        let mut hasher = Blake2s::new();
        hasher.update(info.address.port().to_le_bytes());
        match info.address.ip() {
            IpAddr::V4(ip) => hasher.update(ip.octets()),
            IpAddr::V6(ip) => hasher.update(ip.octets()),
        };
        let a: [u8; 32] = hasher
            .finalize()
            .as_slice()
            .try_into()
            .expect("Wrong length");
        let mut x = vec![0u8; 16];
        x.clone_from_slice(&a[..16]);
        x.try_into().expect("Wrong length")
    }
}