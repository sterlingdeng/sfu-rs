use std::sync::Arc;

use anyhow::{anyhow, Result};

use log;
use tokio::sync::mpsc;
use uuid::Uuid;

use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::APIBuilder;
use webrtc::interceptor::registry::Registry;

use log::error;
use parking_lot::Mutex;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::{OnTrackHdlrFn, RTCPeerConnection};
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::TrackLocal;

use crate::signal::messages;

use super::downtrack::Downtrack;

pub type Id = String;

pub struct Peer {
    id: Id,
    pc: Arc<RTCPeerConnection>,

    pending_candidates: Mutex<Vec<RTCIceCandidateInit>>,
    event_tx: mpsc::Sender<messages::Event>,
}

impl Peer {
    pub async fn new(event_tx: mpsc::Sender<messages::Event>) -> Result<Arc<Self>> {
        let config = RTCConfiguration::default();
        let mut m = MediaEngine::default();
        m.register_default_codecs()?;

        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut m)?;

        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
            .build();

        let pc = Arc::new(api.new_peer_connection(config).await?);

        Ok(Arc::new(Peer {
            id: Id::from(Uuid::new_v4()),
            pending_candidates: Mutex::new(vec![]),
            event_tx,
            pc,
        }))
    }

    pub fn set_on_track_handler_fn(&self, handler: OnTrackHdlrFn) {
        self.pc.on_track(handler)
    }

    pub fn id(&self) -> Id {
        self.id.clone()
    }

    pub(crate) fn pc(&self) -> Arc<RTCPeerConnection> {
        self.pc.clone()
    }

    pub(crate) async fn add_track(&self, track: Arc<Downtrack>) -> Result<()> {
        let transceiver = self.pc.add_transceiver_from_track(track.clone(), None).await?;
        track.set_transceiver(transceiver.clone());

        // need to renegotiate

        tokio::spawn(async move {
            // add cancellation
            loop {
                match transceiver.receiver().await.read_rtcp().await {
                    Ok((_pkts, _attr)) => {}
                    Err(e) => {
                        error!("error reading rtcp for peer: {}", e);
                    }
                };
            }
        });

        Ok(())
    }

    // create_offer is used when the sfu peer connection wants to generate an offer
    // an example is when adding a new media track from another peer.
    pub async fn create_offer(&self) -> Result<RTCSessionDescription> {
        let offer = self.pc.create_offer(None).await?;
        let mut gather_complete = self.pc.gathering_complete_promise().await;
        self.pc.set_local_description(offer).await?;
        let _ = gather_complete.recv().await;
        match self.pc.local_description().await {
            None => Err(anyhow!("failed to get local description")),
            Some(desc) => Ok(desc),
        }
    }

    // set_answer is used when the client returns an answer when responding to an offer
    // created using `create_offer`.
    pub async fn set_answer(&self, answer: RTCSessionDescription) -> Result<()> {
        self.pc.set_remote_description(answer).await?;
        while let Some(candidate) = self.pending_candidates.lock().pop() {
            self.pc.add_ice_candidate(candidate).await?
        }
        Ok(())
    }

    // set_offer_get_answer is used when the client sends an offer. an example
    // usage is a client wants to add a new media track.
    pub async fn set_offer_get_answer(
        &self,
        offer: RTCSessionDescription,
    ) -> Result<RTCSessionDescription> {
        self.pc.set_remote_description(offer).await?;

        let answer = self.pc.create_answer(None).await?;
        self.pc.set_local_description(answer).await?;

        self.pc
            .local_description()
            .await
            .ok_or(anyhow!("failed to get answer"))
    }

    pub async fn add_ice_candidate(&self, candidate: RTCIceCandidateInit) -> Result<()> {
        match self.pc.remote_description().await {
            None => {
                self.pending_candidates.lock().push(candidate);
            }
            Some(_) => self.pc.add_ice_candidate(candidate).await?,
        }
        Ok(())
    }

    pub fn shutdown(&self) {}
}
