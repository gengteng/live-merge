use serde::{Deserialize, Serialize};

/// 视频处理方式
#[derive(Debug, Deserialize, Serialize)]
pub enum FilterMode {
    /// 遮幅，可能有黑边
    Fit,

    /// 剪裁，剪掉不符合长宽比例的部分
    Crop,

    /// 拉伸，拉伸到设定比例
    Scale,
}

/// 视频位置
#[derive(Debug, Deserialize, Serialize)]
pub struct VideoPosition {
    /// 层
    pub layer: u32,

    /// 横坐标
    pub x: u32,

    /// 纵坐标
    pub y: u32,

    /// 宽
    pub width: u32,

    /// 高
    pub height: u32,

    /// 视频处理方式
    pub mode: FilterMode,

    /// ID
    pub id: String,
}
