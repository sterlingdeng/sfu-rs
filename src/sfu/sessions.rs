use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;
use uuid::Uuid;

use crate::sfu::router::Router;
use crate::signal::messages;

use super::*;

pub type Id = String;

pub struct Session {
    mutex: Mutex<()>,
    id: Id,

    router: Arc<Router>,
}

impl Session {
    pub fn new() -> Self {
        Session {
            mutex: Mutex::new(()),
            id: Id::from(Uuid::new_v4()),
            router: Arc::new(Router::new()),
        }
    }

    pub fn id(&self) -> Id {
        self.id.clone()
    }

    pub async fn create_peer(
        &self,
        peer_id: peer::Id,
        event_tx: futures_channel::mpsc::Sender<Result<messages::Event>>,
    ) -> Result<Arc<peer::Peer>> {
        let _ = self.mutex.lock();

        let router = self.router.clone();

        let peer = peer::Peer::new(event_tx).await?;

        router.add_new_peer(peer.clone()).await?;

        peer.set_on_track_handler_fn(Box::new(move |track, rtp_receiver, _| {
            let peer_id = peer_id.clone();
            let track = track.clone();
            let rtp_receiver = rtp_receiver.clone();
            let router = router.clone();
            tokio::spawn(async move {
                let _ = router
                    .on_new_track_remote(peer_id.clone(), track, rtp_receiver)
                    .await;
            });
            Box::pin(async {})
        }));

        return Ok(peer);
    }

    pub async fn get_peer(&self, peer_id: peer::Id) -> Option<Arc<peer::Peer>> {
        let _ = self.mutex.lock();
        self.router.get_peer(peer_id).await?.into()
    }

    pub async fn remove_peer(&self, peer_id: peer::Id) -> Result<()> {
        let _ = self.mutex.lock();
        self.router.remove_peer(peer_id).await;
        Ok(())
    }

    pub fn shutdown(&self) {}
}
