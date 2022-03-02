use bytes::{BufMut, BytesMut};
// use ffmpeg::filter::Graph;
// use ffmpeg::{Filter, Frame, Rational};
// use ffmpeg_sys_next::*;
use openh264::encoder::{EncoderConfig, FrameType};
use openh264::formats::YUVSource;
// use photon_rs::native::save_image;
// use photon_rs::PhotonImage;
use crate::h264::H264Data;
// use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{Receiver, UnboundedSender};

// const INPUT_FORMAT: AVPixelFormat = AVPixelFormat::AV_PIX_FMT_YUV420P;

enum State {
    Created,
    HeaderReceived(openh264::encoder::Encoder),
}

impl Default for State {
    fn default() -> Self {
        Self::Created
    }
}

pub fn decode(
    mut receiver: Receiver<H264Data>,
    h264_sender: UnboundedSender<H264Data>,
) -> anyhow::Result<()> {
    let mut decoder = openh264::decoder::Decoder::new()?;

    let mut state = State::default();

    // let config = FilterConfig::default();
    // let mut graph = build_filter_chain(&config)?;

    // let mut frame = build_frame(&config);

    const CODEC_ID: u8 = 7;

    // padding: 15360
    // 153600
    // 138240
    let mut count = 0;
    while let Some(packet) = receiver.blocking_recv() {
        match packet {
            H264Data::Configuration { raw, record } => {
                let yuv = decoder.decode(raw.as_ref())?;
                log::info!(
                    "{count}) width: {}, height: {}, strides: {:?}, y: {}, u: {}, v: {}, yuv: {}",
                    yuv.width(),
                    yuv.height(),
                    yuv.strides_yuv(),
                    yuv.y().len(),
                    yuv.u().len(),
                    yuv.v().len(),
                    yuv.y().len() + yuv.u().len() + yuv.v().len()
                );
                count += 1;
                let mut buffer = BytesMut::new();
                buffer.put_u8(0x17);
                buffer.put_u32(0);
                record.write_to(&mut buffer);
                let buffer = buffer.freeze();
                if h264_sender.send(H264Data::data(0, buffer)).is_err() {
                    log::error!("Rtmp client closed while sending h264 configuration");
                    break;
                }
            }
            H264Data::Data {
                data: packet,
                timestamp,
            } => {
                log::info!(
                    "Depacketized H264 data({}B): {:02x?}",
                    packet.len(),
                    packet.as_ref()
                );
                let yuv = decoder.decode(packet.as_ref())?;
                log::info!(
                    "{count}) width: {}, height: {}, strides: {:?}, y: {}, u: {}, v: {}, yuv: {}",
                    yuv.width(),
                    yuv.height(),
                    yuv.strides_yuv(),
                    yuv.y().len(),
                    yuv.u().len(),
                    yuv.v().len(),
                    yuv.y().len() + yuv.u().len() + yuv.v().len()
                );
                count += 1;

                if yuv.width() == 0 {
                    continue;
                }

                let mut vec = Vec::with_capacity(1024);

                if let State::Created = state {
                    log::info!(
                        "Got video info: width={}, height={}",
                        yuv.width(),
                        yuv.height()
                    );
                    let config = EncoderConfig::new(yuv.width() as u32, yuv.height() as u32);
                    let mut encoder = openh264::encoder::Encoder::with_config(config)?;

                    let encoded_frame = encoder.encode(&yuv)?;

                    let frame_type = match encoded_frame.frame_type() {
                        FrameType::IDR | FrameType::I => 1,
                        FrameType::P => 2,
                        _ => continue,
                    };
                    vec.put_u8(frame_type << 4 | CODEC_ID);
                    vec.put_slice(&[1, 0, 0, 0]);

                    if encoded_frame.frame_type() == FrameType::IDR {
                        let layer = encoded_frame.layer(1).unwrap();

                        for n in 0..layer.nal_count() {
                            let nal = layer.nal_unit(n).unwrap();
                            vec.put_slice(nal);
                        }
                    }

                    log::info!("h264 sequence header({}B): {:02x?}", vec.len(), vec);
                    if h264_sender
                        .send(H264Data::data(timestamp, vec.into()))
                        .is_err()
                    {
                        log::error!("Rtmp client closed");
                        break;
                    }

                    state = State::HeaderReceived(encoder);
                } else if let State::HeaderReceived(encoder) = &mut state {
                    let encoded_frame = encoder.encode(&yuv)?;
                    let frame_type = match encoded_frame.frame_type() {
                        FrameType::IDR | FrameType::I => 1,
                        FrameType::P => 2,
                        _ => continue,
                    };
                    let codec_id = 7;
                    let packet_type: u8 = 1;
                    let composition_time: i32 = 0; //if frame_type == 1 { 1 } else { 0 };
                    let end = ((packet_type as i32) << 24) | composition_time;
                    vec.put_u8(frame_type << 4 | codec_id);
                    vec.put_i32(end);
                    encoded_frame.write_vec(&mut vec);
                    log::info!(
                        "h264 frame type: {:?}, {} bytes(hex): {:02x?}",
                        encoded_frame.frame_type(),
                        vec.len(),
                        vec
                    );
                    if h264_sender
                        .send(H264Data::data(timestamp, vec.into()))
                        .is_err()
                    {
                        log::error!("Rtmp client closed while sending h264 data");
                        break;
                    }
                }

                // let pts = if start == 0 {
                //     start = ffmpeg::util::time::current();
                //     0
                // } else {
                //     ffmpeg::util::time::current() - start
                // };
                //
                // frame.set_pts(Some(pts));
                // fill_frame(&yuv, &mut frame);
                // graph
                //     .get("in")
                //     .expect("Cannot get filter 'in'")
                //     .source()
                //     .add(&frame)?;
                // graph
                //     .get("out")
                //     .expect("Cannot get filter 'out'")
                //     .sink()
                //     .frame(&mut frame)?;
                // log::info!("[filtered frame] pts: {:?}", frame.pts());
            }
        }
    }

    Ok(())
}
//
// #[allow(dead_code)]
// fn build_frame(config: &FilterConfig) -> Frame {
//     unsafe {
//         let mut frame = Frame::empty();
//         let av_frame = frame.as_mut_ptr();
//         (*av_frame).width = config.width;
//         (*av_frame).height = config.height;
//         (*av_frame).format = INPUT_FORMAT as i32;
//         assert_eq!(av_frame_get_buffer(av_frame, 1), 0);
//         assert_eq!(av_frame_make_writable(av_frame), 0);
//         frame
//     }
// }
//
// #[allow(dead_code)]
// fn fill_frame<YUV: YUVSource>(yuv: &YUV, frame: &mut Frame) {
//     unsafe {
//         let av_frame = frame.as_mut_ptr();
//
//         av_image_copy_plane(
//             (*av_frame).data[0],
//             (*av_frame).linesize[0],
//             yuv.y().as_ptr(),
//             yuv.y_stride(),
//             yuv.width(),
//             yuv.height(),
//         );
//         av_image_copy_plane(
//             (*av_frame).data[1],
//             (*av_frame).linesize[1],
//             yuv.u().as_ptr(),
//             yuv.u_stride(),
//             yuv.width() / 2,
//             yuv.height(),
//         );
//         av_image_copy_plane(
//             (*av_frame).data[2],
//             (*av_frame).linesize[2],
//             yuv.v().as_ptr(),
//             yuv.v_stride(),
//             yuv.width() / 2,
//             yuv.height(),
//         );
//     }
// }
//
// #[derive(Debug, Deserialize, Serialize)]
// pub struct FilterConfig {
//     width: i32,
//     height: i32,
//     #[serde(with = "rational_serde")]
//     time_base: Rational,
// }
//
// impl Default for FilterConfig {
//     fn default() -> Self {
//         Self {
//             width: 320,
//             height: 240,
//             time_base: Rational::new(1, 90000),
//         }
//     }
// }
//
// #[allow(dead_code)]
// pub fn build_filter_chain(config: &FilterConfig) -> anyhow::Result<Graph> {
//     let buffer = find_filter("buffer")?;
//     let scale = find_filter("scale")?;
//     let overlay = find_filter("overlay")?;
//     let crop = find_filter("crop")?;
//     let buffer_sink = find_filter("buffersink")?;
//
//     println!(
//         "filters: {}, {}, {}, {}, {}",
//         buffer.name(),
//         scale.name(),
//         overlay.name(),
//         crop.name(),
//         buffer_sink.name()
//     );
//
//     let mut graph = ffmpeg::filter::Graph::new();
//
//     let buffer_args = format!(
//         "video_size={}x{}:pix_fmt={}:time_base={}:pixel_aspect={}/{}",
//         config.width, config.height, INPUT_FORMAT as i32, config.time_base, 1, 1
//     );
//
//     graph.add(&buffer, "in", &buffer_args)?;
//     graph.add(&buffer_sink, "out", "")?;
//
//     graph.output("in", 0)?.input("out", 0)?.parse("null")?;
//     graph.validate()?;
//
//     Ok(graph)
// }

// #[allow(dead_code)]
// pub fn find_filter(name: &str) -> anyhow::Result<Filter> {
//     ffmpeg::filter::find(name).ok_or_else(|| anyhow::anyhow!("Filter '{}' not found", name))
// }
//
// mod rational_serde {
//     use ffmpeg::Rational;
//     use serde::de::Error;
//     use serde::{Deserialize, Deserializer, Serializer};
//
//     pub fn serialize<S>(rational: &Rational, serializer: S) -> Result<S::Ok, S::Error>
//     where
//         S: Serializer,
//     {
//         let s = format!("{}", rational);
//         serializer.serialize_str(&s)
//     }
//
//     // The signature of a deserialize_with function must follow the pattern:
//     //
//     //    fn deserialize<'de, D>(D) -> Result<T, D::Error>
//     //    where
//     //        D: Deserializer<'de>
//     //
//     // although it may also be generic over the output types T.
//     pub fn deserialize<'de, D>(deserializer: D) -> Result<Rational, D::Error>
//     where
//         D: Deserializer<'de>,
//     {
//         let s = String::deserialize(deserializer)?;
//         let a = s.split('/').collect::<Vec<&str>>();
//         if a.len() != 2 {
//             return Err(D::Error::custom("Invalid rational string"));
//         }
//         match (a[0].parse::<i32>(), a[1].parse::<i32>()) {
//             (Ok(n), Ok(d)) => Ok(Rational::new(n, d)),
//             _ => return Err(D::Error::custom("Failed to parse integer")),
//         }
//     }
// }
