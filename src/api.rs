use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone)]
pub struct ApiClient {
    host: String,
    port: u16,
    http: reqwest::Client,
}

#[derive(Debug, Serialize)]
pub struct PlayParam {
    pub api: String,
    #[serde(rename = "clientip")]
    pub client_ip: Option<IpAddr>,
    pub sdp: String,
    #[serde(rename = "streamurl")]
    pub stream_url: String,
    pub tid: String,
}

#[derive(Debug, Deserialize)]
pub struct PlayResult {
    pub code: i32,
    pub server: Option<String>,
    pub sdp: String,
    #[serde(rename = "sessionid")]
    pub session_id: String,
}

impl ApiClient {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
            http: reqwest::ClientBuilder::new()
                .danger_accept_invalid_certs(true)
                .build()
                .expect("Reqwest client error"),
        }
    }

    pub fn api_url(&self) -> String {
        format!("https://{}:{}/rtc/v1/play/", self.host, self.port)
    }

    pub async fn play(&self, param: &PlayParam) -> anyhow::Result<PlayResult> {
        let resp = self.http.post(self.api_url()).json(param).send().await?;

        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Err(anyhow::anyhow!(
                "Error response: {} {}",
                resp.status(),
                resp.text().await?
            ))
        }
    }
}
