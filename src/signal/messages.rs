use serde::{Deserialize, Serialize};
use webrtc::{
    ice_transport::ice_candidate::RTCIceCandidateInit,
    peer_connection::sdp::session_description::RTCSessionDescription,
};

#[derive(Debug)]
pub enum Event {
    JoinRequest(JoinRequest),
    TrickleIce(TrickleNotification),
    ClientOffer(ClientOfferRequest),
    SfuOffer(RTCSessionDescription),
    ClientAnswer(RTCSessionDescription),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JoinRequest {
    pub session_id: String,
    pub offer: RTCSessionDescription,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct JoinResponse {
    pub answer: RTCSessionDescription,
    pub session_id: String,
    pub peer_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ClientOfferRequest {
    pub session_id: String,
    pub peer_id: String,
    pub offer: RTCSessionDescription,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ClientOfferResponse {
    pub answer: RTCSessionDescription,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TrickleNotification {
    pub target: u32,
    pub candidate: TrickleCandidate,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TrickleCandidate {
    pub candidate: String,
    #[serde(rename = "sdpMid")]
    pub sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex")]
    pub sdp_mline_index: u32,
}

impl From<RTCIceCandidateInit> for TrickleCandidate {
    fn from(t: RTCIceCandidateInit) -> Self {
        TrickleCandidate {
            candidate: t.candidate,
            sdp_mid: t.sdp_mid,
            sdp_mline_index: t.sdp_mline_index.unwrap() as u32,
        }
    }
}

impl From<TrickleCandidate> for RTCIceCandidateInit {
    fn from(t: TrickleCandidate) -> Self {
        RTCIceCandidateInit {
            candidate: t.candidate,
            sdp_mid: t.sdp_mid,
            sdp_mline_index: Some(t.sdp_mline_index as u16),
            username_fragment: None,
        }
    }
}
