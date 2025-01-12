// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.
//
// Copyright (c) DUSK NETWORK. All rights reserved.

use crate::transport::{encoding::Configurable, Encoder};

use crate::encoding::{message::Message, payload::BroadcastPayload};

const DEFAULT_MIN_REPAIR_PACKETS_PER_BLOCK: u32 = 5;
const DEFAULT_MTU: u16 = 1300;
const DEFAULT_FEQ_REDUNDANCY: f32 = 0.15;

use raptorq::Encoder as ExtEncoder;
use serde_derive::{Deserialize, Serialize};

pub struct RaptorQEncoder {
    conf: RaptorQEncoderConf,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct RaptorQEncoderConf {
    min_repair_packets_per_block: u32,
    mtu: u16,
    fec_redundancy: f32,
}

impl Default for RaptorQEncoderConf {
    fn default() -> Self {
        RaptorQEncoderConf {
            fec_redundancy: DEFAULT_FEQ_REDUNDANCY,
            min_repair_packets_per_block: DEFAULT_MIN_REPAIR_PACKETS_PER_BLOCK,
            mtu: DEFAULT_MTU,
        }
    }
}

impl Configurable for RaptorQEncoder {
    type TConf = RaptorQEncoderConf;

    fn default_configuration() -> Self::TConf {
        RaptorQEncoderConf::default()
    }
    fn configure(conf: &Self::TConf) -> Self {
        Self { conf: *conf }
    }
}

impl Encoder for RaptorQEncoder {
    fn encode<'msg>(&self, msg: Message) -> Vec<Message> {
        if let Message::Broadcast(header, payload) = msg {
            let encoder =
                ExtEncoder::with_defaults(&payload.gossip_frame, self.conf.mtu);
            let mut transmission_info =
                encoder.get_config().serialize().to_vec();

            let mut base_packet = payload.generate_uid().to_vec();
            base_packet.append(&mut transmission_info);

            let mut repair_packets =
                (payload.gossip_frame.len() as f32 * self.conf.fec_redundancy
                    / self.conf.mtu as f32) as u32;
            if repair_packets < self.conf.min_repair_packets_per_block {
                repair_packets = self.conf.min_repair_packets_per_block
            }

            encoder
                .get_encoded_packets(repair_packets)
                .iter()
                .map(|encoded_packet| {
                    let mut packet_with_uid = base_packet.clone();
                    packet_with_uid.append(&mut encoded_packet.serialize());
                    Message::Broadcast(
                        header,
                        BroadcastPayload {
                            height: payload.height,
                            gossip_frame: packet_with_uid,
                        },
                    )
                })
                .collect()
        } else {
            vec![msg]
        }
    }
}
