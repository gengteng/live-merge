use ffmpeg::filter::Graph;
use ffmpeg::{Filter, Frame, Rational};
use ffmpeg_sys_next::*;
use serde::{Deserialize, Serialize};

use bytes::Bytes;
use ffmpeg::codec::Id;
use openh264::formats::YUVSource;
use tokio::sync::mpsc::Receiver;

const INPUT_FORMAT: AVPixelFormat = AVPixelFormat::AV_PIX_FMT_YUV420P;

pub fn decode(mut receiver: Receiver<Bytes>) -> anyhow::Result<()> {
    let mut decoder = openh264::decoder::Decoder::new()?;

    let config = FilterConfig::default();
    let mut graph = build_filter_chain(&config)?;

    let mut frame = build_frame(&config);
    let mut start = 0;

    // padding: 15360
    // 153600
    // 138240
    let mut count = 0;
    while let Some(packet) = receiver.blocking_recv() {
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

        let pts = if start == 0 {
            start = ffmpeg::util::time::current();
            0
        } else {
            ffmpeg::util::time::current() - start
        };

        frame.set_pts(Some(pts));
        fill_frame(&yuv, &mut frame);
        graph
            .get("in")
            .expect("Cannot get filter 'in'")
            .source()
            .add(&frame)?;
        graph
            .get("out")
            .expect("Cannot get filter 'out'")
            .sink()
            .frame(&mut frame)?;
        log::info!("[filtered frame] pts: {:?}", frame.pts());
    }

    Ok(())
}

fn build_frame(config: &FilterConfig) -> Frame {
    unsafe {
        let mut frame = Frame::empty();
        let av_frame = frame.as_mut_ptr();
        (*av_frame).width = config.width;
        (*av_frame).height = config.height;
        (*av_frame).format = INPUT_FORMAT as i32;
        assert_eq!(av_frame_get_buffer(av_frame, 1), 0);
        assert_eq!(av_frame_make_writable(av_frame), 0);
        frame
    }
}

fn fill_frame<YUV: YUVSource>(yuv: &YUV, frame: &mut Frame) {
    unsafe {
        let av_frame = frame.as_mut_ptr();

        av_image_copy_plane(
            (*av_frame).data[0],
            (*av_frame).linesize[0],
            yuv.y().as_ptr(),
            yuv.y_stride(),
            yuv.width(),
            yuv.height(),
        );
        av_image_copy_plane(
            (*av_frame).data[1],
            (*av_frame).linesize[1],
            yuv.u().as_ptr(),
            yuv.u_stride(),
            yuv.width() / 2,
            yuv.height(),
        );
        av_image_copy_plane(
            (*av_frame).data[2],
            (*av_frame).linesize[2],
            yuv.v().as_ptr(),
            yuv.v_stride(),
            yuv.width() / 2,
            yuv.height(),
        );
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct FilterConfig {
    width: i32,
    height: i32,
    #[serde(with = "rational_serde")]
    time_base: Rational,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            width: 320,
            height: 240,
            time_base: Rational::new(1, 90000),
        }
    }
}

pub fn build_filter_chain(config: &FilterConfig) -> anyhow::Result<Graph> {
    let buffer = find_filter("buffer")?;
    let scale = find_filter("scale")?;
    let overlay = find_filter("overlay")?;
    let crop = find_filter("crop")?;
    let buffer_sink = find_filter("buffersink")?;

    println!(
        "filters: {}, {}, {}, {}, {}",
        buffer.name(),
        scale.name(),
        overlay.name(),
        crop.name(),
        buffer_sink.name()
    );

    let mut graph = ffmpeg::filter::Graph::new();

    let buffer_args = format!(
        "video_size={}x{}:pix_fmt={}:time_base={}:pixel_aspect={}/{}",
        config.width, config.height, INPUT_FORMAT as i32, config.time_base, 1, 1
    );

    graph.add(&buffer, "in", &buffer_args)?;
    graph.add(&buffer_sink, "out", "")?;

    graph.output("in", 0)?.input("out", 0)?.parse("null")?;
    graph.validate()?;

    Ok(graph)
}

pub fn find_filter(name: &str) -> anyhow::Result<Filter> {
    ffmpeg::filter::find(name).ok_or_else(|| anyhow::anyhow!("Filter '{}' not found", name))
}

pub fn build_audio_encoder() -> anyhow::Result<()> {
    let aac = ffmpeg::codec::encoder::find(Id::AAC)
        .ok_or_else(|| anyhow::anyhow!("Id::AAC not found"))?;
    log::info!("find by id: {}", aac.name());
    let aac = ffmpeg::codec::encoder::find_by_name("libfdk_aac")
        .ok_or_else(|| anyhow::anyhow!("Codec 'libfdk_aac' not found"))?;
    log::info!("find by name: {}", aac.name());
    Ok(())
}

mod rational_serde {
    use ffmpeg::Rational;
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(rational: &Rational, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = format!("{}", rational);
        serializer.serialize_str(&s)
    }

    // The signature of a deserialize_with function must follow the pattern:
    //
    //    fn deserialize<'de, D>(D) -> Result<T, D::Error>
    //    where
    //        D: Deserializer<'de>
    //
    // although it may also be generic over the output types T.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Rational, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let a = s.split('/').collect::<Vec<&str>>();
        if a.len() != 2 {
            return Err(D::Error::custom("Invalid rational string"));
        }
        match (a[0].parse::<i32>(), a[1].parse::<i32>()) {
            (Ok(n), Ok(d)) => Ok(Rational::new(n, d)),
            _ => return Err(D::Error::custom("Failed to parse integer")),
        }
    }
}
