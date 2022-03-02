#![allow(dead_code)]

use crate::api::ApiClient;
use crate::h264::AVCDecoderConfigurationRecord;
use crate::{H264Data, PlayParam};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Sender;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS};
use webrtc::api::APIBuilder;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::sdp_type::RTCSdpType;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
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

pub async fn init(
    sender: Sender<H264Data>,
    host: String,
    port: u16,
    tid: String,
) -> anyhow::Result<Arc<RTCPeerConnection>> {
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

    let offer = peer_connection.create_offer(None).await?;
    peer_connection.set_local_description(offer.clone()).await?;
    log::info!("local description:\n {}", offer.sdp);

    let client = ApiClient::new(&host, port);
    let param = PlayParam {
        api: client.api_url(),
        client_ip: None,
        sdp: offer.sdp,
        stream_url: format!("webrtc://{}/live/{}", host, tid),
        tid,
    };
    let play = client.play(&param).await?;
    log::info!("remote description:\n {}", play.sdp);

    const PROFILE_PREFIX: &'static str = "profile-level-id=";
    let profile_level_id = match play.sdp.find(PROFILE_PREFIX) {
        None => anyhow::bail!("Failed to get profile-level-id"),
        Some(index) => {
            let start = index + PROFILE_PREFIX.len();
            &play.sdp[start..start + 6]
        }
    };

    let profile_indication = u8::from_str_radix(&profile_level_id[..2], 16)?;
    let level_indication = u8::from_str_radix(&profile_level_id[4..], 16)?;

    let record = AVCDecoderConfigurationRecord::new(profile_indication, level_indication);

    log::info!("AVCDecoderConfigurationRecord: {:0x?}", record);

    let pc = Arc::downgrade(&peer_connection);
    peer_connection
        .on_track(Box::new(
            move |track: Option<Arc<TrackRemote>>, receiver: Option<Arc<RTCRtpReceiver>>| {
                let s = sender.clone();
                let pc = pc.clone();
                let r = record.clone();
                Box::pin(async move {
                    if let Some(track) = track {
                        let s = s.clone();
                        let pc = pc.clone();
                        let r = r.clone();
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
                                    loop {
                                        match pc.upgrade() {
                                            None => {
                                                log::error!(
                                                    "Failed to upgrade weak peer connection"
                                                );
                                            }
                                            Some(pc) => {
                                                if let Err(e) = pc
                                                    .write_rtcp(&[Box::new(
                                                        PictureLossIndication {
                                                            sender_ssrc: 0,
                                                            media_ssrc: ssrc,
                                                        },
                                                    )])
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

                                        tokio::time::sleep(Duration::from_secs(3)).await;
                                    }
                                });
                                let mut rtp_decoder = H264Packet::default();
                                let mut has_key_frame = false;
                                while let Ok((packet, _attr)) = track.read_rtp().await {
                                    if !has_key_frame {
                                        has_key_frame = is_key_frame(&packet.payload);
                                        if !has_key_frame {
                                            continue;
                                        }

                                        let config = match rtp_decoder.depacketize(&packet.payload) {
                                            Ok(bytes) => bytes,
                                            Err(e) => {
                                                log::error!("depacketize first key frame error: {}", e);
                                                break;
                                            }
                                        };

                                        // 第一帧
                                        // NAL Header: 0x78 = 0b01111000
                                        // F|NRI|Type
                                        // 0|11 |11000 = 24 => STAP-A
                                        // first RTP payload(23B, hex):
                                        // [78, 00, 0e, 67, 42, c0, 15, 8c, 8d, 40, a0, f9, 00, f0, 88, 46, a0, 00, 04, 68, ce, 3c, 80]
                                        // |HDR|length|UHDR|                       data                        |length| HDR|   data   |
                                        //        14
                                        // Nal unit header: 0x67 = 0b01100111
                                        // F|NRI|Type
                                        // 0|11 |00111 = 7 => sequence parameter sets
                                        //
                                        // https://blog.csdn.net/lu_embedded/article/details/69666414
                                        if let [0x78, body @ ..] = packet.payload.as_ref() {
                                            log::info!("Got a key frame payload in STAP-A format: {:02x?}", body);
                                            let mut rest = body;
                                            let mut record = r.clone();
                                            while let [len0, len1, body @ ..] = rest {
                                                let len =
                                                    u16::from_be_bytes([*len0, *len1]);
                                                if len as usize > body.len() {
                                                    log::error!(
                                                        "Insufficient nal unit data, len={}, data length={}",
                                                        len,
                                                        body.len()
                                                    );
                                                    break;
                                                }

                                                rest = &body[len as usize..];

                                                let data = Vec::from(&body[..len as usize]);
                                                match body {
                                                    [0x67, ..] => record.add_sps(data),
                                                    [0x68, ..] => record.add_pps(data),
                                                    _ => continue,
                                                }
                                            }

                                            if s.send(H264Data::configuration(config, record))
                                                .await
                                                .is_err()
                                            {
                                                log::error!(
                                                        "Failed to send h264 configuration to ffmpeg"
                                                    );
                                                break;
                                            }

                                        } else {
                                            log::error!("First key frame is not a STAP-A packet.");
                                            break;
                                        }
                                    } else {
                                        // 0x7c == 0b01111100, Type = 28, FU-A
                                        // RTP payload format for h264: https://datatracker.ietf.org/doc/html/rfc6184#page-12
                                        log::info!(
                                            "RTP payload({}B): {:02x?}",
                                            packet.payload.len(),
                                            packet.payload.as_ref()
                                        );

                                        match rtp_decoder.depacketize(&packet.payload) {
                                            Ok(h264_pkt) => {
                                                if !h264_pkt.is_empty() {
                                                    let timestamp =
                                                        packet.header.timestamp / clock_rate;
                                                    if s.send(H264Data::data(timestamp, h264_pkt))
                                                        .await
                                                        .is_err()
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

    let mut answer = RTCSessionDescription::default();
    answer.sdp_type = RTCSdpType::Answer;
    answer.sdp = play.sdp;
    peer_connection.set_remote_description(answer).await?;

    let mut gather_complete = peer_connection.gathering_complete_promise().await;
    let _ = gather_complete.recv().await;

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
