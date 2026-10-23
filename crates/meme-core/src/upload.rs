use crate::{
    config::Config,
    error::Error,
    model::{self, names, valid_filename},
    policy::{self, Acceptance},
};
use axum::{extract::Multipart, http::StatusCode};
use image::{ImageFormat, ImageReader};
use std::io::Cursor;

pub struct Upload {
    pub name: String,
    pub display_name: String,
    pub filename: String,
    pub bytes: Vec<u8>,
    pub policy_acceptance: Option<Acceptance>,
}
#[derive(Default)]
struct Input {
    name: Option<String>,
    display_name: Option<String>,
    consent: Option<String>,
    policy_version: Option<String>,
    policy_claims: Option<String>,
    file: Option<(String, Vec<u8>)>,
}
pub async fn read(mut form: Multipart, config: &Config) -> Result<Upload, Error> {
    let mut input = Input::default();
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|_| Error::bad("Invalid upload form"))?
    {
        accept(&mut input, field, config.max_upload_bytes).await?;
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
    let policy_acceptance = policy::accept(
        config,
        input.policy_version.as_deref(),
        input.policy_claims.as_deref(),
    )?;
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
        policy_acceptance,
    })
}
async fn accept(
    input: &mut Input,
    field: axum::extract::multipart::Field<'_>,
    max: usize,
) -> Result<(), Error> {
    let key = field.name().unwrap_or("").to_string();
    if key == "file" {
        input.file = Some(read_file(field, max, input.file.is_some()).await?);
        return Ok(());
    }
    let value = field
        .text()
        .await
        .map_err(|_| Error::bad("Unable to read the form field"))?;
    let limit = if key == "policy_claims" { 1024 } else { 512 };
    if value.len() > limit {
        return Err(Error::bad("Form field is too long"));
    }
    text_field(input, &key, value)
}
fn text_field(input: &mut Input, key: &str, value: String) -> Result<(), Error> {
    let slot = text_slot(input, key)?;
    if slot.is_some() {
        return Err(Error::bad("Duplicate form field"));
    }
    *slot = Some(value);
    Ok(())
}
fn text_slot<'a>(input: &'a mut Input, key: &str) -> Result<&'a mut Option<String>, Error> {
    match key {
        "name" => Ok(&mut input.name),
        "display_name" => Ok(&mut input.display_name),
        "consent" => Ok(&mut input.consent),
        "policy_version" => Ok(&mut input.policy_version),
        "policy_claims" => Ok(&mut input.policy_claims),
        _ => Err(Error::bad("Unknown form field")),
    }
}
async fn read_file(
    field: axum::extract::multipart::Field<'_>,
    max: usize,
    duplicate: bool,
) -> Result<(String, Vec<u8>), Error> {
    if duplicate {
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
    Ok((filename, bytes.to_vec()))
}
pub fn validate_image(bytes: &[u8], filename: &str, max_pixels: u64) -> Result<(), Error> {
    let format =
        image::guess_format(bytes).map_err(|_| Error::bad("The file is not a supported image"))?;
    if model::mime(filename) != Some(image_mime(format)?) {
        return Err(Error::bad("File extension does not match the image format"));
    }
    decode(bytes, format, max_pixels)
}
fn image_mime(format: ImageFormat) -> Result<&'static str, Error> {
    match format {
        ImageFormat::Png => Ok("image/png"),
        ImageFormat::Jpeg => Ok("image/jpeg"),
        ImageFormat::Gif => Ok("image/gif"),
        ImageFormat::WebP => Ok("image/webp"),
        _ => Err(Error::bad("Unsupported image format")),
    }
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
