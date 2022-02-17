mod api;
mod ff;
mod param;
mod rtc;

use crate::api::PlayParam;
use crate::ff::build_audio_encoder;
use bytes::Bytes;
use clap::Parser;
use std::path::PathBuf;
use webrtc::peer_connection::sdp::sdp_type::RTCSdpType;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

#[derive(Debug, Parser)]
struct Opts {
    #[clap(short = 'o', long)]
    output: PathBuf,

    #[clap(short = 'h', long)]
    host: String,

    #[clap(short = 'p', long, default_value = "443")]
    port: u16,

    #[clap(short = 't', long)]
    tid: String,

    #[clap(short = 'l', long, default_value = "INFO")]
    log_level: log::LevelFilter,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Opts {
        host,
        port,
        tid,
        log_level,
        ..
    } = Opts::parse();

    ffmpeg::init()?;

    env_logger::builder().filter(None, log_level).init();

    build_audio_encoder()?;

    let (sender, receiver) = tokio::sync::mpsc::channel::<Bytes>(32);

    tokio::task::spawn_blocking(move || {
        if let Err(e) = ff::decode(receiver) {
            log::error!("ff::decode error: {}", e);
        }
    });

    // Err(anyhow::anyhow!("return"))?;

    let pc = rtc::init(sender).await?;
    let offer = pc.create_offer(None).await?;
    pc.set_local_description(offer.clone()).await?;
    log::info!("local description:\n {}", offer.sdp);

    let client = api::ApiClient::new(&host, port);
    let param = PlayParam {
        api: client.api_url(),
        client_ip: None,
        sdp: offer.sdp,
        stream_url: format!("webrtc://{}/live/{}", host, tid),
        tid,
    };
    let play = client.play(&param).await?;
    log::info!("remote description:\n {}", play.sdp);
    let mut answer = RTCSessionDescription::default();
    answer.sdp_type = RTCSdpType::Answer;
    answer.sdp = play.sdp;
    pc.set_remote_description(answer).await?;

    let mut gather_complete = pc.gathering_complete_promise().await;
    let _ = gather_complete.recv().await;

    std::future::pending::<()>().await;

    Ok(())
}
