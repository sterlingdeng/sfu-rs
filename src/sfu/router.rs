use std::collections::HashMap;

use anyhow::anyhow;
use anyhow::Result;
use log::error;
use std::sync::Arc;
use tokio::sync::Mutex;
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::track::track_remote::TrackRemote;

use crate::sfu::peer;
use crate::sfu::track_handler;

use super::downtrack::Downtrack;
use super::track_handler::TrackHandler;

pub struct Router {
    peers: Arc<Mutex<HashMap<peer::Id, Arc<peer::Peer>>>>,
    // peer_id -> track id -> track_handler
    peer_to_track_handlers:
        Arc<Mutex<HashMap<peer::Id, HashMap<String, Arc<track_handler::TrackHandler>>>>>,
}

impl Router {
    pub fn new() -> Self {
        Router {
            peers: Arc::new(Mutex::new(HashMap::new())),
            peer_to_track_handlers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // add_new_peer adds a new peer and subscribes them to the other tracks
    pub async fn add_new_peer(&self, peer: Arc<peer::Peer>) -> Result<()> {
        let mut guard = self.peer_to_track_handlers.lock().await;
        let peer_id = peer.id().clone();
        if let Some(_) = guard.get(&peer_id) {
            return Err(anyhow!("peer already exists"));
        }
        guard.insert(peer_id, HashMap::new());

        let mut peers_guard = self.peers.lock().await;
        peers_guard.insert(peer.id().clone(), peer.clone());

        Ok(())
    }

    pub async fn remove_peer(&self, peer_id: peer::Id) {
        let mut guard = self.peer_to_track_handlers.lock().await;
        if let Some(tracks) = guard.remove(&peer_id) {
            for (_id, track_handler) in tracks {
                track_handler.close();
            }
        }

        let mut peers_guard = self.peers.lock().await;
        peers_guard.remove(&peer_id);
    }

    pub async fn get_peer(&self, peer_id: peer::Id) -> Option<Arc<peer::Peer>>{
        let peers_guard = self.peers.lock().await;
        match peers_guard.get(&peer_id) {
            Some(v) => {Some(v.clone())},
            None =>  None
        }
    }

    // a callback used to create a new track handler based on a peer's TrackRemote
    pub async fn on_new_track_remote(
        &self,
        peer_id: peer::Id,
        track: Arc<TrackRemote>,
        rtp_receiver: Arc<RTCRtpReceiver>,
    ) -> Result<()> {
        let track_handler = TrackHandler::new(track.clone(), rtp_receiver);
        let track_id = track.id();

        {
            let mut map = self.peer_to_track_handlers.lock().await;

            if let Some(v) = map.get_mut(&peer_id) {
                v.insert(track_id, track_handler.clone());
            } else {
                error!("peer_id {} not initialized in map", peer_id.clone())
            }
        }

        for (other_peer_id, peer) in self.peers.lock().await.iter() {
            if *other_peer_id == peer_id {
                continue;
            }

            let downtrack = Downtrack::new(
                track.id(),
                track.stream_id(),
                peer_id.clone(),
                track.codec().capability,
                track.ssrc(),
            );

            let _ = peer.add_track(downtrack.clone()).await;
            track_handler.add_downtrack(other_peer_id.to_string(), downtrack.clone());
        }
        Ok(())
    }
}
