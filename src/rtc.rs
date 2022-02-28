#![allow(dead_code)]

use crate::H264Data;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS};
use webrtc::api::APIBuilder;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use webrtc::rtp::codecs::h264::H264Packet;
use webrtc::rtp::packetizer::Depacketizer;
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};
use webrtc::rtp_transceiver::rtp_receiver::RTCRtpReceiver;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;
use webrtc::track::track_remote::TrackRemote;

pub async fn init(sender: Sender<H264Data>) -> anyhow::Result<Arc<RTCPeerConnection>> {
    let mut me = MediaEngine::default();
    me.register_codec(
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "".to_owned(),
                rtcp_feedback: vec![],
            },
            payload_type: 102,
            ..Default::default()
        },
        RTPCodecType::Video,
    )?;
    me.register_codec(
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                ..Default::default()
            },
            payload_type: 120,
            ..Default::default()
        },
        RTPCodecType::Audio,
    )?;

    let mut registry = Registry::new();

    // Use the default set of Interceptors
    registry = register_default_interceptors(registry, &mut me)?;

    // Create the API object with the MediaEngine
    let api = APIBuilder::new()
        .with_media_engine(me)
        .with_interceptor_registry(registry)
        .build();

    // Prepare the configuration
    let config = RTCConfiguration::default();

    // Create a new RTCPeerConnection
    let peer_connection = Arc::new(api.new_peer_connection(config).await?);

    // Allow us to receive 1 audio track, and 1 video track
    peer_connection
        .add_transceiver_from_kind(
            RTPCodecType::Audio,
            &[RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Recvonly,
                send_encodings: vec![],
            }],
        )
        .await?;
    peer_connection
        .add_transceiver_from_kind(
            RTPCodecType::Video,
            &[RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Recvonly,
                send_encodings: vec![],
            }],
        )
        .await?;

    let pc = Arc::downgrade(&peer_connection);
    peer_connection
        .on_track(Box::new(
            move |track: Option<Arc<TrackRemote>>, receiver: Option<Arc<RTCRtpReceiver>>| {
                let s = sender.clone();
                let pc = pc.clone();
                Box::pin(async move {
                    if let Some(track) = track {
                        let s = s.clone();
                        let pc = pc.clone();
                        tokio::spawn(async move {
                            let codec = track.codec().await;
                            let mime_type = codec.capability.mime_type;
                            let clock_rate = codec.capability.clock_rate;
                            let ssrc = track.ssrc();
                            log::info!(
                                "[on_track] ssrc: {}, payload_type: {}",
                                track.ssrc(),
                                track.payload_type()
                            );
                            if mime_type.starts_with("video") {
                                tokio::spawn(async move {
                                    match pc.upgrade() {
                                        None => {
                                            log::error!("Failed to upgrade weak peer connection");
                                        }
                                        Some(pc) => {
                                            if let Err(e) = pc
                                                .write_rtcp(&[Box::new(PictureLossIndication {
                                                    sender_ssrc: 0,
                                                    media_ssrc: ssrc,
                                                })])
                                                .await
                                            {
                                                log::error!(
                                                    "Send pic loss indication error: {}",
                                                    e
                                                );
                                            }
                                            log::info!("Send pic los indication");
                                        }
                                    }
                                });
                                let mut rtp_decoder = H264Packet::default();
                                let mut has_key_frame = false;
                                while let Ok((packet, attr)) = track.read_rtp().await {
                                    log::info!(
                                        "header: {:?}, attributes: {:?}",
                                        packet.header,
                                        attr
                                    );
                                    // log::info!(
                                    //     "[{}:{}] {} bytes received.",
                                    //     ssrc,
                                    //     mime_type,
                                    //     packet.payload.len()
                                    // );
                                    if !has_key_frame {
                                        has_key_frame = is_key_frame(&packet.payload);
                                        if !has_key_frame {
                                            continue;
                                        }

                                        log::info!("got a key frame");
                                    }
                                    match rtp_decoder.depacketize(&packet.payload) {
                                        Ok(h264_pkt) => {
                                            if !h264_pkt.is_empty() {
                                                let timestamp =
                                                    packet.header.timestamp / clock_rate;
                                                if let Err(_) =
                                                    s.send(H264Data::new(timestamp, h264_pkt)).await
                                                {
                                                    log::error!(
                                                        "Failed to send h264 packet to ffmpeg"
                                                    );
                                                    break;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            log::error!(
                                                "Failed to depacketize rtp packet to h264: {}",
                                                e
                                            );
                                            break;
                                        }
                                    }
                                }
                            } else {
                                while let Ok((_packet, _)) = track.read_rtp().await {
                                    // log::info!(
                                    //     "[{}:{}] {} bytes received.",
                                    //     ssrc,
                                    //     mime_type,
                                    //     packet.payload.len()
                                    // );
                                }
                            }
                        });
                    }
                    if let Some(receiver) = receiver {
                        log::info!(
                            "[on_track] receiver kind: {}, parameters: {:?}",
                            receiver.kind(),
                            receiver.get_parameters().await
                        );
                    }
                })
            },
        ))
        .await;

    peer_connection
        .on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
            Box::pin(async move {
                log::info!("[on_peer_connection_state_change] {}", state);
            })
        }))
        .await;

    peer_connection
        .on_ice_candidate(Box::new(move |candidate| {
            Box::pin(async move { log::info!("[on_ice_candidate] {:?}", candidate) })
        }))
        .await;

    peer_connection
        .on_ice_connection_state_change(Box::new(move |state| {
            Box::pin(async move { log::info!("[on_ice_connection_state_change] {}", state) })
        }))
        .await;

    peer_connection
        .on_data_channel(Box::new(move |channel| {
            Box::pin(async move {
                log::info!(
                    "[on_data_channel] id: {}, label: {}",
                    channel.id(),
                    channel.label()
                )
            })
        }))
        .await;

    peer_connection
        .on_signaling_state_change(Box::new(move |state| {
            Box::pin(async move {
                log::info!("[on_signaling_state_change] {}", state);
            })
        }))
        .await;

    Ok(peer_connection)
}

const NALU_TTYPE_STAP_A: u32 = 24;
const NALU_TTYPE_SPS: u32 = 7;
const NALU_TYPE_BITMASK: u32 = 0x1F;

fn is_key_frame(data: &[u8]) -> bool {
    if data.len() < 4 {
        false
    } else {
        let word = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let nalu_type = (word >> 24) & NALU_TYPE_BITMASK;
        (nalu_type == NALU_TTYPE_STAP_A && (word & NALU_TYPE_BITMASK) == NALU_TTYPE_SPS)
            || (nalu_type == NALU_TTYPE_SPS)
    }
}
