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
        .map_err(|_| Error::bad("Invalid upload form"))?
    {
        accept(&mut input, field, max).await?;
    }
    let name = input.name.ok_or_else(|| Error::bad("Name is required"))?;
    let display_name = input
        .display_name
        .ok_or_else(|| Error::bad("Display name is required"))?;
    if input.consent.as_deref() != Some("true") {
        return Err(Error::bad(
            "Please consent to a public submission and GitHub coauthor attribution",
        ));
    }
    names(&name, &display_name)?;
    let (filename, bytes) = input
        .file
        .ok_or_else(|| Error::bad("Please select an image"))?;
    if !valid_filename(&filename) {
        return Err(Error::bad(
            "Filenames may contain only ASCII letters, digits, dots, underscores, and hyphens",
        ));
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
            return Err(Error::bad("Each meme must contain exactly one image"));
        }
        let filename = field.file_name().unwrap_or("").to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|_| Error::bad("Unable to read the image"))?;
        if bytes.is_empty() || bytes.len() > max {
            return Err(Error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Image size exceeds the limit".into(),
            ));
        }
        input.file = Some((filename, bytes.to_vec()));
        return Ok(());
    }
    let value = field
        .text()
        .await
        .map_err(|_| Error::bad("Unable to read the form field"))?;
    if value.len() > 512 {
        return Err(Error::bad("Form field is too long"));
    }
    text_field(input, &key, value)
}
fn text_field(input: &mut Input, key: &str, value: String) -> Result<(), Error> {
    let slot = match key {
        "name" => &mut input.name,
        "display_name" => &mut input.display_name,
        "consent" => &mut input.consent,
        _ => return Err(Error::bad("Unknown form field")),
    };
    if slot.is_some() {
        return Err(Error::bad("Duplicate form field"));
    }
    *slot = Some(value);
    Ok(())
}
pub fn validate_image(bytes: &[u8], filename: &str, max_pixels: u64) -> Result<(), Error> {
    let format =
        image::guess_format(bytes).map_err(|_| Error::bad("The file is not a supported image"))?;
    let expected = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        _ => return Err(Error::bad("Unsupported image format")),
    };
    if model::mime(filename) != Some(expected) {
        return Err(Error::bad("File extension does not match the image format"));
    }
    decode(bytes, format, max_pixels)
}
fn decode(bytes: &[u8], format: ImageFormat, max_pixels: u64) -> Result<(), Error> {
    let dimensions = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| Error::bad("Image header is corrupt"))?;
    if u64::from(dimensions.0) * u64::from(dimensions.1) > max_pixels {
        return Err(Error::bad("Image pixel count exceeds the limit"));
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|_| Error::bad("Image is corrupt or exceeds decoding resource limits"))?;
    Ok(())
}
