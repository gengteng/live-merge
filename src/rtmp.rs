use crate::h264::H264Data;
use anyhow::bail;
use rml_rtmp::handshake::{HandshakeProcessResult, PeerType};
use rml_rtmp::sessions::{
    ClientSession, ClientSessionConfig, ClientSessionEvent, ClientSessionResult,
    PublishRequestType, StreamMetadata,
};
use rml_rtmp::time::RtmpTimestamp;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub struct RtmpConnection {
    sender: UnboundedSender<Command>,
}

impl RtmpConnection {
    pub async fn connect<A: ToSocketAddrs>(
        addr: A,
        mut h264_reader: UnboundedReceiver<H264Data>,
    ) -> anyhow::Result<Self> {
        let mut socket = TcpStream::connect(addr).await?;

        let mut handshake = rml_rtmp::handshake::Handshake::new(PeerType::Client);
        let p0_and_p1 = handshake.generate_outbound_p0_and_p1()?;

        socket.write_all(&p0_and_p1).await?;
        socket.flush().await?;

        let buffer = loop {
            let mut buffer = [0u8; 8192];
            let bytes =
                tokio::time::timeout(Duration::from_secs(3), socket.read(&mut buffer)).await??;
            match handshake.process_bytes(&buffer[..bytes])? {
                HandshakeProcessResult::InProgress { response_bytes } => {
                    if !response_bytes.is_empty() {
                        socket.write_all(&response_bytes).await?;
                        socket.flush().await?;
                    }
                }
                HandshakeProcessResult::Completed {
                    response_bytes,
                    remaining_bytes,
                } => {
                    if !response_bytes.is_empty() {
                        socket.write_all(&response_bytes).await?;
                    }
                    break remaining_bytes;
                }
            }
        };

        let mut config = ClientSessionConfig::new();
        config.tc_url = Some(format!("rtmp://localhost/live"));
        let (mut session, results) = ClientSession::new(config)?;

        for result in results {
            if let ClientSessionResult::OutboundResponse(packet) = result {
                socket.write_all(&packet.bytes).await?;
            }
        }

        let results = session.handle_input(&buffer)?;

        for result in results {
            if let ClientSessionResult::OutboundResponse(packet) = result {
                socket.write_all(&packet.bytes).await?;
            }
        }

        socket.flush().await?;

        let (sender, mut receiver) = unbounded_channel();

        tokio::spawn(async move {
            let mut buffer = [0u8; 8192];
            loop {
                tokio::select! {
                    result = socket.read(&mut buffer) => {
                        let bytes = result?;
                        if bytes == 0 {
                            log::error!("read 0 bytes");
                            break;
                        }

                        let results = session.handle_input(&buffer[0..bytes])?;

                        for result in &results {
                            match result {
                                ClientSessionResult::OutboundResponse(packet) => socket.write_all(&packet.bytes).await?,
                                ClientSessionResult::RaisedEvent(event) => {
                                    match event {
                                        ClientSessionEvent::ConnectionRequestAccepted => {
                                            RtmpConnection::send_outbound_packet(&mut socket, session.request_publishing("gengteng".to_string(), PublishRequestType::Live)?).await?;
                                        }
                                        ClientSessionEvent::PublishRequestAccepted => {
                                            let mut metadata = StreamMetadata::new();
                                            metadata.encoder = Some(format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")));
                                            metadata.video_width = Some(320);
                                            metadata.video_height = Some(240);
                                            metadata.video_codec = Some("7".to_string());
                                            RtmpConnection::send_outbound_packet(&mut socket, session.publish_metadata(&metadata)?).await?;
                                            // RtmpConnection::send_outbound_packet(&mut socket, session.publish_video_data(Bytes::new(), RtmpTimestamp::new(0), true)?).await?;
                                            // RtmpConnection::send_outbound_packet(&mut socket, session.publish_audio_data(Bytes::new(), RtmpTimestamp::new(0), true)?).await?;
                                            let mut timestamp = RtmpTimestamp::new(0);
                                            let mut rtp_ts = None;
                                            while let Some(h264) = h264_reader.recv().await {
                                                match rtp_ts {
                                                    Some(ts) => {
                                                        let offset = h264.timestamp() - ts;
                                                        timestamp = timestamp + offset;
                                                        rtp_ts = Some(ts + offset);
                                                    }
                                                    None => {
                                                        rtp_ts = Some(h264.timestamp());
                                                    }
                                                }
                                                RtmpConnection::send_outbound_packet(&mut socket, session.publish_video_data(h264.into(), timestamp, true)?).await?;
                                            }
                                        }
                                        e => log::info!("event: {:?}", e),
                                    }
                                },
                                ClientSessionResult::UnhandleableMessageReceived(message) => {
                                    log::info!("unhandleable message received: {}", message.message_stream_id);
                                }
                            }
                        }

                        if !results.is_empty() {
                            socket.flush().await?;
                        }
                    }

                    option = receiver.recv() => {
                        match option {
                            Some(cmd) => {
                                match cmd {
                                    Command::Publish { app } => {
                                        if let ClientSessionResult::OutboundResponse(packet) = session.request_connection(app)? {
                                            socket.write_all(&packet.bytes).await?;
                                            socket.flush().await?;
                                        }
                                    }
                                }
                            }
                            None => {
                                log::info!("session closed");
                                break;
                            }
                        }
                    }
                }
            }

            Ok::<_, anyhow::Error>(())
        });

        Ok(RtmpConnection { sender })
    }

    pub async fn publish(&mut self, app_name: &str) -> anyhow::Result<()> {
        if self
            .sender
            .send(Command::Publish {
                app: app_name.to_string(),
            })
            .is_err()
        {
            bail!("Failed to publish, channel closed.");
        }

        Ok(())
    }

    async fn send_outbound_packet(
        socket: &mut TcpStream,
        result: ClientSessionResult,
    ) -> anyhow::Result<()> {
        match result {
            ClientSessionResult::OutboundResponse(packet) => {
                socket.write_all(&packet.bytes).await?;
                socket.flush().await?;
                Ok(())
            }
            result => bail!(
                "Client session result is not outbound response: {:?}",
                result
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    Publish { app: String },
}
