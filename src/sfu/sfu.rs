use super::sessions;

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Sfu {
    sessions: Mutex<HashMap<sessions::Id, Arc<sessions::Session>>>,
}

impl Sfu {
    pub fn new() -> Arc<Self> {
        Arc::new(Sfu {
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn get_session(&self, id: sessions::Id) -> Option<Arc<sessions::Session>> {
        let sessions = self.sessions.lock();
        match sessions.get(&id) {
            Some(session) => return Some(session.clone()),
            None => None,
        }
    }

    pub fn new_session(&self) -> Arc<sessions::Session> {
        let mut sessions = self.sessions.lock();
        let new_session = sessions::Session::new();
        let session = sessions
            .entry(new_session.id().clone())
            .or_insert(Arc::new(new_session));

        session.clone()
    }

    pub fn shutdown(&self) {
        todo!()
    }
}
