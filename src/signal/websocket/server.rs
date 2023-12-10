use anyhow::Result;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use std::sync::Arc;
// wtf is adding this to get split
use futures_util::{
    future,
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt, TryStreamExt,
};

use log::{error, info, warn};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::{
    sfu::{sessions::Session, sfu::Sfu},
    signal::{self, messages::TrickleNotification},
};
use tokio_tungstenite::{accept_async, WebSocketStream};

use super::models::{JsonRpcKind, Notification, Response};

pub struct Server<'a> {
    addr: &'a str,
    sfu: Arc<Sfu>,

    // waitgroup?
    cancellation: CancellationToken,
}

impl<'a> Server<'a> {
    pub fn new(addr: &'a str, sfu: Arc<Sfu>) -> Self {
        Server {
            addr,
            sfu,
            cancellation: CancellationToken::new(),
        }
    }

    // todo: use rocket here later
    pub async fn listen_and_serve(&self) -> anyhow::Result<()> {
        let listener: TcpListener;
        match TcpListener::bind(self.addr).await {
            Ok(l) => listener = l,
            Err(e) => return Err(anyhow::anyhow!("failed to bind tcp listener: {}", e)),
        };

        info!("listening on {}", listener.local_addr().unwrap());

        loop {
            let sfu = self.sfu.clone();
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            tokio::spawn(async move {
                                handler(stream, sfu.clone()).await;
                            });
                        },
                        Err(e) => {
                            error!("error accepting TCP connection: {}", e);
                        },
                    }
                }
                _ = self.cancellation.cancelled() => {
                    break
                },

            };
        }
        // listener doesn't need to be explicitly closed when it gets dropped?
        Ok(())
    }

    async fn shutdown(&self) {}
}

fn jsonrpc_shim_layer(
    writer: SplitSink<WebSocketStream<TcpStream>, tungstenite::Message>,
    reader: SplitStream<WebSocketStream<TcpStream>>,
) -> (
    futures_channel::mpsc::Receiver<Result<JsonRpcKind>>,
    futures_channel::mpsc::Sender<Result<JsonRpcKind>>,
) {
    let (request_sink, request_reader) = futures_channel::mpsc::channel::<Result<JsonRpcKind>>(64);
    let (response_sink, response_reader) =
        futures_channel::mpsc::channel::<Result<JsonRpcKind>>(64);

    tokio::spawn(async move {
        let inbound_fut = reader
            .map_err(|err| error!("websocket read err: {}", err))
            .try_filter(|msg| future::ready(msg.is_text()))
            .map(|msg| serde_json::from_str::<JsonRpcKind>(msg.unwrap().to_text().unwrap()))
            .map_err(|err| {
                error!("error parsing json: {}", err);
                err.into()
            })
            .map_ok(|msg| {
                info!("got message: {:?}", msg);
                msg
            })
            .filter(|r| future::ready(r.is_ok()))
            .map(Ok)
            .forward(request_sink);

        let outbound_fut = response_reader
            .map_ok(|evt| serde_json::to_value(&evt).unwrap())
            .map_err(|err| error!("error reading from request reader: {}", err))
            // missing jsonrpc here
            .map_ok(|v| serde_json::to_string(&v).unwrap())
            .map_ok(|msg| tungstenite::Message::from(msg))
            .map_err(|_| tungstenite::error::Error::ConnectionClosed)
            .forward(writer);

        tokio::select! {
            _ = inbound_fut => {
                info!("websocket closed")
            }
            _ = outbound_fut => {
                info!("client closed")
            }
        }
    });

    (request_reader, response_sink)
}

async fn handler(stream: TcpStream, sfu: Arc<Sfu>) {
    let ws_stream = accept_async(stream).await.expect("Failed to accept");
    let (ws_writer, ws_reader) = ws_stream.split();

    let (rpc_reader, rpc_writer) = jsonrpc_shim_layer(ws_writer, ws_reader);

    message_event_loop(rpc_reader, rpc_writer, sfu).await;

    // give writer and reader to a shim layer that converts WS messages <> signal::events
}

async fn message_event_loop(
    mut receiver: futures_channel::mpsc::Receiver<Result<JsonRpcKind>>,
    mut rpc_write_stream: futures_channel::mpsc::Sender<Result<JsonRpcKind>>,
    sfu: Arc<Sfu>,
) {
    let (mut sfu_signal_writer, mut sfu_signal_reader) =
        futures_channel::mpsc::channel::<Result<signal::messages::Event>>(64);

    let mut rpc_write_stream2 = rpc_write_stream.clone();
    tokio::spawn(async move {
        // receive json rpc and convert to a signal event.
        while let Some(Ok(evt)) = receiver.next().await {
            match evt {
                JsonRpcKind::Request(req) => match req.method.as_str() {
                    "join" => {
                        info!("received join request");
                        let id = req.id;
                        let join_request = serde_json::from_value::<signal::messages::JoinRequest>(
                            serde_json::to_value(req.params).unwrap(),
                        )
                        .expect("failed to unmarshal json");

                        let session: Arc<Session>;
                        if let Some(s) = sfu.get_session(join_request.session_id) {
                            session = s;
                        } else {
                            session = sfu.new_session();
                        }

                        let peer_id = uuid::Uuid::new_v4().to_string();
                        let peer = session
                            .create_peer(peer_id.clone(), sfu_signal_writer.clone())
                            .await
                            .expect("failed to create peer");

                        let answer = peer
                            .set_offer_get_answer(join_request.offer)
                            .await
                            .expect("failed to get answer");

                        let _ = rpc_write_stream2
                            .send(Ok(JsonRpcKind::Response(Response {
                                id,
                                result: serde_json::from_value(
                                    serde_json::to_value(signal::messages::JoinResponse {
                                        answer,
                                        session_id: session.id().clone(),
                                        peer_id: peer_id.to_string(),
                                    })
                                    .unwrap(),
                                )
                                .unwrap(),
                                error: None,
                            })))
                            .await
                            .unwrap();
                    }
                    "offer" => {
                        info!("received offer request");
                        let id = req.id;
                        let client_offer_request = serde_json::from_value::<signal::messages::ClientOfferRequest>(
                            serde_json::to_value(req.params).unwrap(),
                        )
                        .expect("failed to unmarshal json");

                        let session = sfu
                            .get_session(client_offer_request.session_id.to_string())
                            .expect("failed to get session");

                        let peer = session.get_peer(client_offer_request.peer_id)
                            .await
                            .expect("failed to get peer");

                        let answer = peer.set_offer_get_answer(client_offer_request.offer)
                            .await
                            .expect("failed to set client offer");
                        
                        rpc_write_stream2
                            .send(Ok(JsonRpcKind::Response(Response {
                                id,
                                result: serde_json::from_value(
                                    serde_json::to_value(signal::messages::ClientOfferResponse {
                                        answer,
                                    })
                                    .unwrap(),
                                )
                                .unwrap(),
                                error: None,
                            })))
                            .await
                            .unwrap();


                    }
                    v => warn!("unknown json rpc request method: {}", v),
                },
                JsonRpcKind::Notification(msg) => match msg.method.as_str() {
                    "trickle" => {
                        let trickle = serde_json::from_value::<TrickleNotification>(
                            serde_json::to_value(msg.params).unwrap(),
                            ).expect("failed to unmarshal json");
                        sfu_signal_writer.send(Ok(signal::messages::Event::TrickleIce(trickle)))
                            .await
                            .expect("failed to send trickle message to sfu");
                    }
                    "answer" => {
                        let answer = serde_json::from_value::<RTCSessionDescription>(
                            serde_json::to_value(msg.params).unwrap(),
                            ).expect("failed to unmarshal json");
                        sfu_signal_writer.send(Ok(signal::messages::Event::ClientAnswer(answer)))
                            .await
                            .expect("failed to send trickle message to sfu");
                    }
                    v => warn!("unknown json rpc request method: {}", v),
                }
                msg => error!("unhandled message from SFU -> client: {:?}", msg),
            }
        }
    });

    tokio::spawn(async move {
        while let Some(Ok(resp)) = sfu_signal_reader.next().await {
            match resp {
                signal::messages::Event::SfuOffer(offer) => {
                    rpc_write_stream.send(Ok(JsonRpcKind::Notification(Notification {
                        method: "offer".to_string(),
                        params: serde_json::from_value(serde_json::to_value(offer).unwrap())
                            .unwrap(),
                    })))
                    .await
                    .expect("failed to send rpc");
                }

                signal::messages::Event::TrickleIce(trickle_ice) => {
                    rpc_write_stream.send(Ok(JsonRpcKind::Notification(Notification {
                        method: "trickle".to_string(),
                        params: serde_json::from_value(serde_json::to_value(trickle_ice).unwrap())
                            .unwrap(),
                    })))
                    .await
                    .expect("failed to send rpc");
                }
                msg => error!("unhandled message from SFU -> client: {:?}", msg),
            }
        }
    });
}
