//! 位图解码：把磁盘上的 PNG/JPEG/WebP 变成 GPU 可直接上传的 RGBA8。

/// 解码后的 RGBA8 位图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// 长度恒为 `width * height * 4`。
    pub rgba: Vec<u8>,
}

impl DecodedImage {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, TextureError> {
        let expected = width as usize * height as usize * 4;
        if width == 0 || height == 0 {
            return Err(TextureError::BadSize { width, height });
        }
        if rgba.len() != expected {
            return Err(TextureError::BadLength { expected, actual: rgba.len() });
        }
        Ok(Self { width, height, rgba })
    }

    /// 纯色图（测试与占位用）。
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..(width as usize * height as usize) {
            data.extend_from_slice(&rgba);
        }
        Self { width, height, rgba: data }
    }

    /// 读取某个像素（越界返回透明黑）。
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0, 0, 0, 0];
        }
        let i = ((y as usize * self.width as usize) + x as usize) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// 上下翻转（本引擎纹理第 0 行是画面顶部，与常见图片格式一致；
    /// 读回渲染结果时用得到）。
    pub fn flip_vertical(&self) -> Self {
        let stride = self.width as usize * 4;
        let mut out = vec![0u8; self.rgba.len()];
        for y in 0..self.height as usize {
            let src = y * stride;
            let dst = (self.height as usize - 1 - y) * stride;
            out[dst..dst + stride].copy_from_slice(&self.rgba[src..src + stride]);
        }
        Self { width: self.width, height: self.height, rgba: out }
    }

    /// 写出 PNG（截图、命令行工具与黄金图测试用）。
    pub fn save_png(&self, path: impl AsRef<std::path::Path>) -> Result<(), TextureError> {
        let buffer = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone())
            .ok_or(TextureError::BadLength {
                expected: self.width as usize * self.height as usize * 4,
                actual: self.rgba.len(),
            })?;
        buffer.save(path.as_ref()).map_err(|e| TextureError::Decode(e.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TextureError {
    #[error("无法解码图片：{0}")]
    Decode(String),
    #[error("图片尺寸非法：{width}x{height}")]
    BadSize { width: u32, height: u32 },
    #[error("RGBA 数据长度不符：期望 {expected} 字节，实际 {actual} 字节")]
    BadLength { expected: usize, actual: usize },
    #[error("纹理尺寸超出设备上限：{width}x{height} > {max}")]
    TooLarge { width: u32, height: u32, max: u32 },
}

/// 解码 PNG/JPEG/WebP 字节流。
pub fn decode_bytes(bytes: &[u8]) -> Result<DecodedImage, TextureError> {
    let image = image::load_from_memory(bytes).map_err(|e| TextureError::Decode(e.to_string()))?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    DecodedImage::new(width, height, rgba.into_raw())
}

/// 读取文件并解码。
pub fn decode_file(path: impl AsRef<std::path::Path>) -> Result<DecodedImage, TextureError> {
    let bytes = std::fs::read(path).map_err(|e| TextureError::Decode(e.to_string()))?;
    decode_bytes(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_and_pixel() {
        let img = DecodedImage::solid(2, 3, [10, 20, 30, 255]);
        assert_eq!(img.rgba.len(), 2 * 3 * 4);
        assert_eq!(img.pixel(1, 2), [10, 20, 30, 255]);
        assert_eq!(img.pixel(9, 9), [0, 0, 0, 0]);
    }

    #[test]
    fn bad_sizes_are_rejected() {
        assert!(DecodedImage::new(0, 4, vec![]).is_err());
        assert!(DecodedImage::new(2, 2, vec![0; 4]).is_err());
        assert!(DecodedImage::new(2, 2, vec![0; 16]).is_ok());
    }

    #[test]
    fn flip_vertical_reverses_rows() {
        let img = DecodedImage::new(
            1,
            2,
            vec![1, 2, 3, 4, 5, 6, 7, 8],
        )
        .unwrap();
        let flipped = img.flip_vertical();
        assert_eq!(flipped.pixel(0, 0), [5, 6, 7, 8]);
        assert_eq!(flipped.pixel(0, 1), [1, 2, 3, 4]);
    }

    #[test]
    fn decode_png_round_trip() {
        // 用内存里编码出来的 PNG 验证解码链路
        let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([9, 8, 7, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let decoded = decode_bytes(&png).unwrap();
        assert_eq!((decoded.width, decoded.height), (4, 2));
        assert_eq!(decoded.pixel(3, 1), [9, 8, 7, 255]);
    }

    #[test]
    fn garbage_bytes_fail_cleanly() {
        let err = decode_bytes(&[0, 1, 2, 3]).unwrap_err();
        assert!(matches!(err, TextureError::Decode(_)));
    }
}
