use anyhow::Result;
use std::sync::Arc;
// wtf is adding this to get split
use futures_util::{
    future,
    stream::{SplitSink, SplitStream},
    StreamExt, TryStreamExt,
};

use log::{error, info, warn};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::{sfu::sfu::Sfu, signal};
use tokio_tungstenite::{accept_async, WebSocketStream};

use super::models::JsonRpcKind;

struct Server<'a> {
    addr: &'a str,
    sfu: Arc<Sfu>,

    cancellation: CancellationToken,
}

impl<'a> Server<'a> {
    fn new(addr: &'a str, sfu: Arc<Sfu>) -> Self {
        Server {
            addr,
            sfu,
            cancellation: CancellationToken::new(),
        }
    }

    async fn handler(&self, stream: TcpStream) {
        let ws_stream = accept_async(stream).await.expect("Failed to accept");
        let (rpc_writer, ws_reader) = ws_stream.split();

        let (signal_writer, rpc_reader) = rpc_shim_layer(rpc_writer, ws_reader);

        // give writer and reader to a shim layer that converts WS messages <> signal::events
    }

    // todo: use rocket here later
    async fn listen_and_serve(&self) -> anyhow::Result<()> {
        let listener: TcpListener;
        match TcpListener::bind(self.addr).await {
            Ok(l) => listener = l,
            Err(e) => return Err(anyhow::anyhow!("failed to bind tcp listener: {}", e)),
        };

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            tokio::spawn(async {
                                self.handler(stream);
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

fn rpc_shim_layer(
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

async fn signal_shim_layer(
    mut receiver: futures_channel::mpsc::Receiver<Result<JsonRpcKind>>,
    sender: futures_channel::mpsc::Sender<Result<JsonRpcKind>>,
) -> (
    futures_channel::mpsc::Receiver<Result<signal::messages::Event>>,
    futures_channel::mpsc::Sender<Result<signal::messages::Event>>,
) {
    let (request_sink, request_reader) =
        futures_channel::mpsc::channel::<Result<signal::messages::Event>>(64);
    let (response_sink, mut response_reader) =
        futures_channel::mpsc::channel::<Result<signal::messages::Event>>(64);

    tokio::spawn(async move {
        // receive json rpc and convert to a signal event.
        while let Some(Ok(evt)) = receiver.next().await {
            match evt {
                JsonRpcKind::Request(req) => match req.method.as_str() {
                    "join" => {}
                    "offer" => {}
                    v => warn!("unknown json rpc request method: {}", v),
                },
                JsonRpcKind::Notification(note) => {}
                _ => {}
            }
        }
    });

    tokio::spawn(async move {
        while let Some(Ok(resp)) = response_reader.next().await {
            match resp {
                // SFU -> client messages
                signal::messages::Event::SfuOffer(offer) => {}
                signal::messages::Event::TrickleIce(trickle_ice) => {}
                _ => {}
            }
        }
    });

    (request_reader, response_sink)
}
