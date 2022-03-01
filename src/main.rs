mod api;
mod codec;
mod h264;
mod param;
mod rtc;
mod rtmp;

use crate::api::PlayParam;
use crate::h264::H264Data;
use crate::rtmp::RtmpConnection;
use clap::Parser;
use tokio::sync::mpsc::unbounded_channel;

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

    // ffmpeg::init()?;

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
        if let Err(e) = codec::decode(receiver, h264_sender) {
            log::error!("ff::decode error: {}", e);
        }
    });

    let _pc = rtc::init(sender, host, port, tid).await?;

    tokio::signal::ctrl_c().await?;

    Ok(())
}
