// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
//
// Copyright (c) DUSK NETWORK. All rights reserved.

use crate::config::BucketConfig;
use crate::K_K;

use super::node::{Node, NodeEvictionStatus};
use super::BinaryKey;
use arrayvec::ArrayVec;
use rand::seq::SliceRandom;
use rand::thread_rng;

pub(super) struct Bucket<V> {
    nodes: arrayvec::ArrayVec<Node<V>, K_K>,
    pending_node: Option<Node<V>>,
    bucket_config: BucketConfig,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeInsertOk<'a, TNode> {
    Inserted {
        inserted: &'a TNode,
    },
    Updated {
        updated: &'a TNode,
        pending_eviction: Option<&'a TNode>,
    },
    Pending {
        pending_insert: &'a TNode,
        pending_eviction: Option<&'a TNode>,
    },
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeInsertError<TNode> {
    Invalid(TNode),
    Full(TNode),
}

impl<'a, TNode> NodeInsertOk<'a, TNode> {
    pub fn pending_eviction(&self) -> Option<&TNode> {
        match self {
            NodeInsertOk::Inserted { inserted: _ } => None,
            NodeInsertOk::Pending {
                pending_insert: _,
                pending_eviction,
            } => *pending_eviction,
            NodeInsertOk::Updated {
                updated: _,
                pending_eviction,
            } => *pending_eviction,
        }
    }
}
pub type InsertOk<'a, V> = NodeInsertOk<'a, Node<V>>;
pub type InsertError<V> = NodeInsertError<Node<V>>;
impl<V> Bucket<V> {
    pub(super) fn new(bucket_config: BucketConfig) -> Self {
        Bucket {
            nodes: ArrayVec::<Node<V>, K_K>::new(),
            pending_node: None,
            bucket_config,
        }
    }

    //Refreshes the node's last usage time corresponding to the given key and
    // return his ref
    fn refresh_node(&mut self, key: &BinaryKey) -> Option<&Node<V>> {
        let old_index =
            self.nodes.iter().position(|s| s.id().as_binary() == key)?;
        self.nodes[old_index..].rotate_left(1);
        self.nodes.last_mut()?.refresh();
        self.nodes.last()
    }

    fn insert_pending(&mut self) {
        if self.nodes.is_full() {
            return;
        };
        if let Some(pending) = self.pending_node.take() {
            if pending.is_alive(self.bucket_config.node_ttl) {
                //FIXME: use try_push instead of push
                //FIXME2: we are breaking the LRU policy, maybe in
                // the meanwhile other records have been updated. Btw
                // it's mitigated with is_alive check
                self.nodes.push(pending);
            }
        }
    }

    /*
        If the bucket is full, flag the least recent used for eviction.
        If it's already flagged, check if timeout is expired and then replace with the pending node.
        The method return the candidate for eviction (if any)
    */
    fn try_perform_eviction(&mut self) -> Option<&Node<V>> {
        if !self.nodes.is_full() {
            return None;
        }
        match self.nodes.first()?.eviction_status {
            NodeEvictionStatus::Requested(instant) => {
                if instant.elapsed() < self.bucket_config.node_evict_after {
                    self.nodes.first()
                } else {
                    self.nodes.pop_at(0);
                    self.insert_pending();
                    None
                }
            }
            NodeEvictionStatus::None => {
                if self.nodes.first()?.is_alive(self.bucket_config.node_ttl) {
                    None
                } else {
                    self.nodes.first_mut()?.flag_for_check();
                    self.nodes.first()
                }
            }
        }
    }

    pub fn insert(
        &mut self,
        node: Node<V>,
    ) -> Result<InsertOk<V>, InsertError<V>> {
        if !node.is_id_valid() {
            return Err(NodeInsertError::Invalid(node));
        }
        if self.refresh_node(node.id().as_binary()).is_some() {
            self.try_perform_eviction();
            return Ok(NodeInsertOk::Updated {
                pending_eviction: self.pending_eviction_node(),
                updated: self.nodes.last().unwrap(),
            });
        }
        self.try_perform_eviction();
        match self.nodes.try_push(node) {
            Ok(_) => Ok(NodeInsertOk::Inserted {
                inserted: self.nodes.last().unwrap(),
            }),
            Err(err) => {
                if self
                    .nodes
                    .first()
                    .expect("Bucket full but no node as .first()")
                    .is_alive(self.bucket_config.node_ttl)
                {
                    Err(NodeInsertError::Full(err.element()))
                } else {
                    self.pending_node = Some(err.element());
                    Ok(NodeInsertOk::Pending {
                        pending_insert: self
                            .pending_node
                            .as_ref()
                            .expect("Unable to get the pending node back"),
                        pending_eviction: self.pending_eviction_node(),
                    })
                }
            }
        }
    }

    //pick at most `ITEM_COUNT` random nodes from this bucket
    pub fn pick<const ITEM_COUNT: usize>(
        &self,
    ) -> impl Iterator<Item = &Node<V>> {
        let mut idxs: Vec<usize> = (0..self.nodes.len()).collect();
        idxs.shuffle(&mut thread_rng());
        idxs.into_iter()
            .take(ITEM_COUNT)
            .filter_map(move |idx| self.nodes.get(idx))
    }

    /* The method return the least recent used node to query if flagged for
     * eviction */
    fn pending_eviction_node(&self) -> Option<&Node<V>> {
        self.nodes
            .first()
            .filter(|n| n.eviction_status != NodeEvictionStatus::None)
    }

    pub(super) fn peers(&self) -> impl Iterator<Item = &Node<V>> {
        self.nodes.iter()
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.nodes.first().map_or(false, |n| {
            n.seen_at.elapsed() > self.bucket_config.bucket_ttl
        })
    }

    pub(crate) fn remove_idle_nodes(&mut self) {
        let ttl = self.bucket_config.node_ttl;
        self.nodes.retain(|n| n.is_alive(ttl));
        self.insert_pending();
    }

    pub(crate) fn alive_nodes(&self) -> impl Iterator<Item = &Node<V>> {
        let ttl = self.bucket_config.node_ttl;
        self.nodes.iter().filter(move |&n| n.is_alive(ttl))
    }

    pub(crate) fn has_node(&self, peer: &BinaryKey) -> bool {
        self.nodes.iter().any(|n| n.id().as_binary() == peer)
    }

    pub(crate) fn is_full(&self) -> bool {
        self.nodes.is_full()
    }
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use crate::{
        config::BucketConfig,
        kbucket::{
            bucket::NodeInsertError, key::BinaryKey, Bucket, Node,
            NodeInsertOk, Tree,
        },
        peer::PeerNode,
        K_BETA,
    };

    impl<V> Bucket<V> {
        pub fn last_id(&self) -> Option<&BinaryKey> {
            self.nodes.last().map(|n| n.id().as_binary())
        }

        pub fn least_used_id(&self) -> Option<&BinaryKey> {
            self.nodes.first().map(|n| n.id().as_binary())
        }

        pub fn remove_id(&mut self, id: &[u8]) -> Option<Node<V>> {
            let update_idx =
                self.nodes.iter().position(|s| s.id().as_binary() == id)?;

            let removed = self.nodes.pop_at(update_idx);
            if let Some(pending) = self.pending_node.take() {
                self.nodes.push(pending);
            }
            removed
        }
    }

    impl<V> crate::kbucket::Tree<V> {
        fn bucket_for_test(&mut self) -> &mut Bucket<V> {
            self.get_or_create_bucket(1)
        }
    }

    #[test]
    fn test_lru_base_5secs() {
        let root = PeerNode::generate("127.0.0.1:666");
        let mut config = BucketConfig::default();
        config.node_evict_after = Duration::from_millis(1000);
        config.node_ttl = Duration::from_secs(5);

        let mut route_table = Tree::new(root, config);

        let bucket = route_table.bucket_for_test();
        let node1 = PeerNode::generate("192.168.1.1:8080");
        let id_node1 = node1.id().as_binary().clone();
        let node1_copy = PeerNode::generate("192.168.1.1:8080");
        match bucket.insert(node1).expect("This should return an ok()") {
            NodeInsertOk::Inserted { .. } => {}
            _ => assert!(false),
        }
        let a = bucket.pick::<K_BETA>();
        assert_eq!(a.count(), 1);

        match bucket
            .insert(node1_copy)
            .expect("This should return an ok()")
        {
            NodeInsertOk::Updated { .. } => {}
            _ => assert!(false),
        }
        assert_eq!(Some(&id_node1), bucket.last_id());
        let node2 = PeerNode::generate("192.168.1.2:8080");
        let id_node2 = node2.id().as_binary().clone();

        match bucket.insert(node2).expect("This should return an ok()") {
            NodeInsertOk::Inserted { inserted: _ } => {}
            _ => assert!(false),
        }
        let a = bucket.pick::<K_BETA>();
        assert_eq!(a.count(), 2);
        assert_eq!(Some(&id_node2), bucket.last_id());
        assert_eq!(Some(&id_node1), bucket.least_used_id());

        match bucket
            .insert(PeerNode::generate("192.168.1.1:8080"))
            .expect("This should return an ok()")
        {
            NodeInsertOk::Updated { .. } => {}
            _ => assert!(false),
        }
        let a = bucket.pick::<K_BETA>();
        assert_eq!(a.count(), 2);
        assert_eq!(Some(&id_node1), bucket.last_id());
        assert_eq!(Some(&id_node2), bucket.least_used_id());
        let a = bucket.remove_id(&id_node2);
        assert_eq!(&id_node2, a.unwrap().id().as_binary());
        assert_eq!(Some(&id_node1), bucket.last_id());
        assert_eq!(Some(&id_node1), bucket.least_used_id());
        for i in 2..21 {
            match bucket
                .insert(PeerNode::generate(
                    &format!("192.168.1.{}:8080", i)[..],
                ))
                .expect("This should return an ok()")
            {
                NodeInsertOk::Inserted { .. } => {
                    assert!(bucket.pick::<K_BETA>().count() <= K_BETA);
                }
                _ => assert!(false),
            }
        }
        assert_eq!(bucket.pick::<K_BETA>().count(), K_BETA);
        let pending = PeerNode::generate("192.168.1.21:8080");
        let pending_id = pending.id().as_binary().clone();
        match bucket.insert(pending).expect_err("this should be error") {
            NodeInsertError::Full(pending) => {
                assert_eq!(pending.id().as_binary(), &pending_id);
                thread::sleep(Duration::from_secs(5));
                match bucket.insert(pending).expect("this should be ok") {
                    NodeInsertOk::Pending {
                        pending_insert,
                        pending_eviction: _,
                    } => {
                        assert_eq!(
                            pending_insert.id().as_binary(),
                            &pending_id
                        );
                        thread::sleep(Duration::from_secs(1));
                        let pending = PeerNode::generate("192.168.1.21:8080");
                        match bucket.insert(pending).expect("this should be ok")
                        {
                            NodeInsertOk::Inserted { inserted: _ } => {}
                            v => {
                                println!("{:?}", v);
                                assert!(false)
                            }
                        }
                    }
                    b => {
                        println!("{:?}", b);
                        assert!(false)
                    }
                }
            }
            _ => assert!(false),
        }
    }
}
