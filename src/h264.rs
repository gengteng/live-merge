use bytes::Bytes;

pub struct H264Data {
    timestamp: u32,
    data: Bytes,
}

impl H264Data {
    pub fn new(timestamp: u32, data: Bytes) -> Self {
        Self { timestamp, data }
    }

    pub fn timestamp(&self) -> u32 {
        self.timestamp
    }

    pub fn data(&self) -> &Bytes {
        &self.data
    }
}

impl From<H264Data> for Bytes {
    fn from(p: H264Data) -> Self {
        p.data
    }
}
