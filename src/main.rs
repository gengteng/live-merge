mod api;
mod ff;
mod h264;
mod param;
mod rtc;
mod rtmp;

use crate::api::PlayParam;
use crate::h264::H264Data;
use crate::rtmp::RtmpConnection;
use clap::Parser;
use tokio::sync::mpsc::unbounded_channel;
use webrtc::peer_connection::sdp::sdp_type::RTCSdpType;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

#[derive(Debug, Parser)]
struct Opts {
    #[clap(short = 'o', long)]
    output: String,

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
        output,
        host,
        port,
        tid,
        log_level,
    } = Opts::parse();

    ffmpeg::init()?;

    env_logger::builder().filter(None, log_level).init();

    let (h264_sender, h264_receiver) = unbounded_channel();

    tokio::spawn(async move {
        if let Err(e) = async move {
            let mut rtmp_conn = RtmpConnection::connect(&output, h264_receiver).await?;
            log::info!("[rtmp handshaked] addr: {}", output,);

            rtmp_conn.publish("gengteng").await?;

            tokio::signal::ctrl_c().await?;

            Ok::<_, anyhow::Error>(())
        }
        .await
        {
            log::error!("Rtmp client error: {}", e);
        }
    });

    let (sender, receiver) = tokio::sync::mpsc::channel::<H264Data>(32);

    tokio::task::spawn_blocking(move || {
        if let Err(e) = ff::decode(receiver, h264_sender) {
            log::error!("ff::decode error: {}", e);
        }
    });

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

    tokio::signal::ctrl_c().await?;

    Ok(())
}
