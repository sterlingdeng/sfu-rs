use std::sync::{atomic, Arc, Mutex};

use bytes::{Buf, BufMut, Bytes, BytesMut};

use anyhow::Result;
use log::info;
use webrtc::{
    rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType},
    track::track_local::{self, TrackLocalWriter},
    util::Unmarshal,
    Error,
};

use super::*;

pub struct Downtrack<'a> {
    id: &'a str,
    stream_id: &'a str,
    peer_id: peer::Id,

    codec: RTCRtpCodecCapability,
    codec_type: RTPCodecType,

    // inner set after bind
    inner: Mutex<DowntrackInner>,

    // internal fields
    publisher_ssrc: u32,
    // rtcp_writer: func([]rtcp.Packet)
    bound: atomic::AtomicBool,
    muted: atomic::AtomicBool,
    resync: atomic::AtomicBool,
}

struct DowntrackInner {
    ssrc: u32,
    payload_type: u8,
    mime_type: String,
    write_stream: Option<Arc<dyn TrackLocalWriter>>,

    last_seq_num: u16,
    seq_num_offset: u16,
}

impl<'a> Downtrack<'a> {
    pub async fn write_rtp(&self, p: &rtp::packet::Packet) -> Result<usize> {
        if !self.bound.load(atomic::Ordering::Relaxed) {
            return Ok(0);
        }
        if !self.muted.load(atomic::Ordering::Relaxed) {
            return Ok(0);
        }
        let mut inner = self.inner.lock().unwrap();
        if self.resync.load(atomic::Ordering::Relaxed) {
            if self.codec_type == RTPCodecType::Video {
                if inner.last_seq_num != 0 {
                    inner.seq_num_offset = p.header.sequence_number - inner.last_seq_num - 1;
                }
                self.resync.store(false, atomic::Ordering::Relaxed);
            }
        }

        // Each downtrack tracks its own sequence number to minimize large gaps when tracks are
        // muted/unmuted.
        // SRTP decryption is stateful and large gaps in sequence numbers will cause unmuted tracks
        // to fail.
        let new_seq_num = p.header.sequence_number - inner.seq_num_offset;
        inner.last_seq_num = new_seq_num;

        let mut header = p.header.clone();
        header.ssrc = inner.ssrc;
        header.payload_type = inner.payload_type;
        header.sequence_number = new_seq_num;

        let mut pkt = p.clone();
        pkt.header = header;

        if let Some(write_stream) = &inner.write_stream {
            match write_stream.write_rtp(&pkt).await {
                Ok(n) => return Ok(n as usize),
                Err(e) => return Err(e.into()),
            };
        }

        return Ok(0);
    }

    /// write encrypts and writes a full RTP packet
    pub async fn write(&self, b: &[u8]) -> Result<usize> {
        let mut buf = BytesMut::new();
        buf.clone_from_slice(b);
        let packet = rtp::packet::Packet::unmarshal(&mut buf)?;
        self.write_rtp(&packet).await
    }
}

impl<'a> track_local::TrackLocal for Downtrack<'a> {
    fn id(&self) -> &str {
        self.id
    }

    fn stream_id(&self) -> &str {
        self.stream_id
    }

    fn kind(&self) -> webrtc::rtp_transceiver::rtp_codec::RTPCodecType {
        self.codec_type
    }

    fn as_any(&self) -> &dyn std::any::Any {
        todo!()
    }

    fn bind<'life0, 'life1, 'async_trait>(
        &'life0 self,
        t: &'life1 track_local::TrackLocalContext,
    ) -> core::pin::Pin<
        Box<
            dyn core::future::Future<
                    Output = webrtc::error::Result<
                        webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecParameters,
                    >,
                > + core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        let parameters = RTCRtpCodecParameters {
            capability: self.codec.clone(),
            ..Default::default()
        };

        match codec_parameter_fuzzy_search(parameters, t.codec_parameters()) {
            Ok(codec) => {
                self.bound.store(true, atomic::Ordering::Relaxed);
                let mut inner = self.inner.lock().unwrap();
                inner.ssrc = t.ssrc();
                inner.payload_type = codec.payload_type;
                inner.mime_type = codec.capability.mime_type.clone();
                inner.write_stream = Some(t.write_stream().unwrap());
                info!("downtrack bound");
                return Box::pin(async { Ok(codec) });
            }
            Err(_) => Box::pin(async { Err(Error::ErrCodecNotFound) }),
        }
    }

    fn unbind<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _t: &'life1 track_local::TrackLocalContext,
    ) -> core::pin::Pin<
        Box<
            dyn core::future::Future<Output = webrtc::error::Result<()>>
                + core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        self.bound.store(false, atomic::Ordering::Relaxed);
        info!("downtrack unbound");
        Box::pin(async { Ok(()) })
    }
}

fn codec_parameter_fuzzy_search(
    needle: RTCRtpCodecParameters,
    haystack: &[RTCRtpCodecParameters],
) -> Result<RTCRtpCodecParameters> {
    for c in haystack.iter() {
        if c.capability.sdp_fmtp_line == needle.capability.sdp_fmtp_line
            && (c
                .capability
                .mime_type
                .eq_ignore_ascii_case(needle.capability.mime_type.as_str()))
        {
            return Ok(c.clone());
        }
    }

    for c in haystack.iter() {
        if c.capability
            .mime_type
            .eq_ignore_ascii_case(needle.capability.mime_type.as_str())
        {
            return Ok(c.clone());
        }
    }

    Err(webrtc::Error::ErrCodecNotFound.into())
}
