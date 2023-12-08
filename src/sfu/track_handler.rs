use log::{error, warn};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Sender;
use tokio_util::sync::CancellationToken;
use webrtc::rtp;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;

use crate::sfu::peer;

use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::{TrackLocal, TrackLocalWriter};
use webrtc::track::track_remote::TrackRemote;

/// TrackHandler struct manages the fanout of a uptrack (TrackRemote) and the downtracks.
pub struct TrackHandler {
    uptrack: Arc<TrackRemote>,
    upstream_rtp_receiver: Arc<RTCRtpReceiver>,
    // peer_id -> track_id -> TrackLocalStaticRTP
    // downtracks: Arc<Mutex<HashMap<peer::Id, HashMap<String, TrackLocalStaticRTP>>>>,
    broadcast_tx: Sender<rtp::packet::Packet>,

    peers: Arc<Mutex<HashMap<peer::Id, ()>>>,

    cancellation: CancellationToken,
}

impl TrackHandler {
    pub fn new(uptrack: Arc<TrackRemote>, rtp_receiver: Arc<RTCRtpReceiver>) -> Arc<Self> {
        let (tx, _rx) = broadcast::channel(1024);

        let uptrack_clone = uptrack.clone();
        let tx_clone = tx.clone();
        let cancellation = CancellationToken::new();

        let token = cancellation.clone();

        let track_handler = Arc::new(TrackHandler {
            uptrack,
            upstream_rtp_receiver: rtp_receiver.clone(),
            broadcast_tx: tx,
            // downtracks: Arc::new(Mutex::new(HashMap::new())),
            peers: Arc::new(Mutex::new(HashMap::new())),
            cancellation: token,
        });

        let token = cancellation.clone();
        tokio::spawn(async move { start_rtp_loop(uptrack_clone, tx_clone, token).await });

        let token = cancellation.clone();
        tokio::spawn(async move { start_rtcp_loop(rtp_receiver, token).await });
        track_handler
    }

    pub fn get_codec_info(&self) -> webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters {
        self.uptrack.codec()
    }

    // todo: downtrack to return an err closed to indicate that it needs to be removed;
    pub fn add_downtrack(&self, peer_id: peer::Id, downtrack: Arc<TrackLocalStaticRTP>) {
        use broadcast::error::RecvError;

        let downtrack = downtrack.clone();
        let mut broadcast_rx = self.broadcast_tx.subscribe();
        if let Some(_) = self.peers.lock().insert(peer_id.clone(), ()) {
            warn!("peer {} already exists in track handler", peer_id.clone());
            return;
        }

        let peers_hm = self.peers.clone();

        tokio::spawn(async move {
            loop {
                match broadcast_rx.recv().await {
                    Ok(packet) => {
                        if let Err(e) = downtrack.write_rtp(&packet).await {
                            // exit early if error, but todo: emit a specific error to indicate unsubscription
                            error!("error writing rtp: {}", e);
                            peers_hm.lock().remove_entry(&peer_id.clone());
                            return;
                        }
                    }
                    Err(e) => match e {
                        RecvError::Closed => return,
                        RecvError::Lagged(_) => {
                            error!("rtp receive channel is lagging")
                        }
                    },
                }
            }
        });
    }

    pub fn close(&self) {
        self.cancellation.cancel();
    }
}

async fn start_rtcp_loop(rtp_receiver: Arc<RTCRtpReceiver>, cancel: CancellationToken) {
    loop {
        tokio::select! {
            v = rtp_receiver.read_rtcp() => {
                match v {
                    Ok((_, _)) => {}
                    Err(e) => {
                        error!("error reading rtcp: {}", e)
                    }
                }
            }
            _ = cancel.cancelled() => {
                return
            }
        }
    }
}

async fn start_rtp_loop(
    uptrack: Arc<TrackRemote>,
    sender: Sender<rtp::packet::Packet>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            Ok((pkt, _attr)) = uptrack.read_rtp() => {
                if sender.receiver_count() > 0 {
                    if let Err(e) = sender.send(pkt) {
                        error!("failed to send RTP: {}", e);
                    }
                }
            }
            _ = cancel.cancelled() => {
               return
            },
        }
    }
}
