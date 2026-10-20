use crate::{
    error::Error,
    model::{self, names, valid_filename},
};
use axum::{extract::Multipart, http::StatusCode};
use image::{ImageFormat, ImageReader};
use std::io::Cursor;

pub struct Upload {
    pub name: String,
    pub display_name: String,
    pub filename: String,
    pub bytes: Vec<u8>,
}
#[derive(Default)]
struct Input {
    name: Option<String>,
    display_name: Option<String>,
    consent: Option<String>,
    file: Option<(String, Vec<u8>)>,
}
pub async fn read(mut form: Multipart, max: usize) -> Result<Upload, Error> {
    let mut input = Input::default();
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|_| Error::bad("上传表单无效"))?
    {
        accept(&mut input, field, max).await?;
    }
    let name = input.name.ok_or_else(|| Error::bad("缺少 Name"))?;
    let display_name = input
        .display_name
        .ok_or_else(|| Error::bad("缺少外显名称"))?;
    if input.consent.as_deref() != Some("true") {
        return Err(Error::bad("请确认公开投稿和 GitHub 共同作者署名"));
    }
    names(&name, &display_name)?;
    let (filename, bytes) = input.file.ok_or_else(|| Error::bad("请选择图片"))?;
    if !valid_filename(&filename) {
        return Err(Error::bad("文件名只支持英文字母、数字、点、下划线和连字符"));
    }
    Ok(Upload {
        name,
        display_name,
        filename,
        bytes,
    })
}
async fn accept(
    input: &mut Input,
    field: axum::extract::multipart::Field<'_>,
    max: usize,
) -> Result<(), Error> {
    let key = field.name().unwrap_or("").to_string();
    if key == "file" {
        if input.file.is_some() {
            return Err(Error::bad("每个表情包只能包含一张图片"));
        }
        let filename = field.file_name().unwrap_or("").to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|_| Error::bad("无法读取图片"))?;
        if bytes.is_empty() || bytes.len() > max {
            return Err(Error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "图片大小超出限制".into(),
            ));
        }
        input.file = Some((filename, bytes.to_vec()));
        return Ok(());
    }
    let value = field
        .text()
        .await
        .map_err(|_| Error::bad("无法读取表单字段"))?;
    if value.len() > 512 {
        return Err(Error::bad("表单字段过长"));
    }
    text_field(input, &key, value)
}
fn text_field(input: &mut Input, key: &str, value: String) -> Result<(), Error> {
    let slot = match key {
        "name" => &mut input.name,
        "display_name" => &mut input.display_name,
        "consent" => &mut input.consent,
        _ => return Err(Error::bad("未知的表单字段")),
    };
    if slot.is_some() {
        return Err(Error::bad("表单字段重复"));
    }
    *slot = Some(value);
    Ok(())
}
pub fn validate_image(bytes: &[u8], filename: &str, max_pixels: u64) -> Result<(), Error> {
    let format = image::guess_format(bytes).map_err(|_| Error::bad("文件不是支持的图片"))?;
    let expected = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        _ => return Err(Error::bad("不支持该图片格式")),
    };
    if model::mime(filename) != Some(expected) {
        return Err(Error::bad("文件扩展名与真实图片格式不符"));
    }
    decode(bytes, format, max_pixels)
}
fn decode(bytes: &[u8], format: ImageFormat, max_pixels: u64) -> Result<(), Error> {
    let dimensions = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| Error::bad("图片头损坏"))?;
    if u64::from(dimensions.0) * u64::from(dimensions.1) > max_pixels {
        return Err(Error::bad("图片像素数超出限制"));
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| Error::bad("图片损坏或解码资源超出限制"))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_spoofed_truncated_and_large_images() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        assert!(validate_image(png.get_ref(), "meme.png", 4).is_ok());
        assert!(validate_image(png.get_ref(), "meme.jpg", 4).is_err());
        assert!(validate_image(png.get_ref(), "meme.png", 1).is_err());
        assert!(validate_image(b"<svg onload='x'>", "meme.png", 100).is_err());
        assert!(validate_image(&png.get_ref()[..16], "meme.png", 100).is_err());
        assert!(!valid_filename("../meme.png"));
    }
}
