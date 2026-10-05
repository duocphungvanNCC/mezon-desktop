use std::collections::HashMap;

use libwebrtc::stats::RtcStats;

const LOSS_RATIO: f64 = 0.05;
const MIN_PACKETS: u64 = 50;
const CLEAR_SAMPLES: u32 = 2;

struct StreamLoss {
    upload: bool,
    packets: u64,
    lost: i64,
}

#[derive(Default)]
struct LossWindow {
    expected: u64,
    lost: u64,
}

impl LossWindow {
    fn is_lossy(&self) -> bool {
        self.expected >= MIN_PACKETS && self.lost as f64 / self.expected as f64 >= LOSS_RATIO
    }
}

#[derive(Default)]
pub(super) struct NetworkQuality {
    streams: HashMap<String, StreamLoss>,
    weak: bool,
    clean_samples: u32,
}

impl NetworkQuality {
    pub(super) fn update(&mut self, stats: &[RtcStats]) -> bool {
        let streams = loss_streams(stats);
        let mut received = LossWindow::default();
        let mut sent = LossWindow::default();
        for (id, stream) in &streams {
            let Some(previous) = self.streams.get(id) else {
                continue;
            };
            let lost = (stream.lost - previous.lost).max(0) as u64;
            let packets = stream.packets.saturating_sub(previous.packets);
            if stream.upload {
                sent.lost += lost;
                sent.expected += packets;
            } else {
                received.lost += lost;
                received.expected += packets + lost;
            }
        }
        self.streams = streams;
        let lossy = received.is_lossy() || sent.is_lossy();
        self.clean_samples = if lossy { 0 } else { self.clean_samples + 1 };
        if lossy {
            self.weak = true;
        } else if self.clean_samples >= CLEAR_SAMPLES {
            self.weak = false;
        }
        self.weak
    }
}

fn loss_streams(stats: &[RtcStats]) -> HashMap<String, StreamLoss> {
    let packets_sent: HashMap<&str, u64> = stats
        .iter()
        .filter_map(|stat| match stat {
            RtcStats::OutboundRtp(out) => Some((out.rtc.id.as_str(), out.sent.packets_sent)),
            _ => None,
        })
        .collect();
    stats
        .iter()
        .filter_map(|stat| match stat {
            RtcStats::InboundRtp(inbound) => Some((
                inbound.rtc.id.clone(),
                StreamLoss {
                    upload: false,
                    packets: inbound.received.packets_received,
                    lost: inbound.received.packets_lost,
                },
            )),
            RtcStats::RemoteInboundRtp(remote) => packets_sent
                .get(remote.remote_inbound.local_id.as_str())
                .map(|&packets| {
                    (
                        remote.rtc.id.clone(),
                        StreamLoss {
                            upload: true,
                            packets,
                            lost: remote.received.packets_lost,
                        },
                    )
                }),
            _ => None,
        })
        .collect()
}
