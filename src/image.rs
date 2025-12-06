use anyhow::{Context, Result};
use bytes::Bytes;
use image::{DynamicImage, GenericImageView, ImageFormat};
use std::io::Cursor;

/// Supported image output formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Avif,
    WebP,
    Jpeg,
    Png,
    Original,
}

/// Image converter for format transformations
pub struct ImageConverter {
    quality: u8,
    max_dimension: u32,
    enable_avif: bool,
    enable_webp: bool,
}

impl ImageConverter {
    pub fn new(quality: u8, max_dimension: u32, enable_avif: bool, enable_webp: bool) -> Self {
        Self {
            quality,
            max_dimension,
            enable_avif,
            enable_webp,
        }
    }
    
    /// Convert image to the requested format
    pub fn convert(&self, data: &Bytes, target_format: OutputFormat) -> Result<(Bytes, &'static str)> {
        // Try to detect and decode the image
        let img = image::load_from_memory(data)
            .context("Failed to decode image")?;
        
        // Check dimensions and resize if necessary
        let img = self.resize_if_needed(img);
        
        // Convert to target format
        let (converted, mime_type) = match target_format {
            OutputFormat::Avif if self.enable_avif => {
                (self.to_avif(&img)?, "image/avif")
            }
            OutputFormat::WebP if self.enable_webp => {
                (self.to_webp(&img)?, "image/webp")
            }
            OutputFormat::Jpeg => {
                (self.to_jpeg(&img)?, "image/jpeg")
            }
            OutputFormat::Png => {
                (self.to_png(&img)?, "image/png")
            }
            OutputFormat::Original => {
                // Return original data
                return Ok((data.clone(), "application/octet-stream"));
            }
            _ => {
                // Fallback to JPEG if format is disabled
                (self.to_jpeg(&img)?, "image/jpeg")
            }
        };
        
        Ok((converted, mime_type))
    }
    
    /// Resize image if it exceeds maximum dimensions
    fn resize_if_needed(&self, img: DynamicImage) -> DynamicImage {
        let (width, height) = img.dimensions();
        
        if width > self.max_dimension || height > self.max_dimension {
            let scale = if width > height {
                self.max_dimension as f32 / width as f32
            } else {
                self.max_dimension as f32 / height as f32
            };
            
            let new_width = (width as f32 * scale) as u32;
            let new_height = (height as f32 * scale) as u32;
            
            img.resize(new_width, new_height, image::imageops::FilterType::Lanczos3)
        } else {
            img
        }
    }
    
    /// Convert image to AVIF format
    fn to_avif(&self, img: &DynamicImage) -> Result<Bytes> {
        let mut buffer = Vec::new();
        let encoder = image::codecs::avif::AvifEncoder::new_with_speed_quality(
            &mut buffer,
            10, // Speed (1-10, 10 is fastest)
            self.quality,
        );
        
        img.write_with_encoder(encoder)
            .context("Failed to encode AVIF")?;
        
        Ok(Bytes::from(buffer))
    }
    
    /// Convert image to WebP format
    fn to_webp(&self, img: &DynamicImage) -> Result<Bytes> {
        let mut buffer = Vec::new();
        let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut buffer);
        
        img.write_with_encoder(encoder)
            .context("Failed to encode WebP")?;
        
        Ok(Bytes::from(buffer))
    }
    
    /// Convert image to JPEG format
    fn to_jpeg(&self, img: &DynamicImage) -> Result<Bytes> {
        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        
        img.write_to(&mut cursor, ImageFormat::Jpeg)
            .context("Failed to encode JPEG")?;
        
        Ok(Bytes::from(buffer))
    }
    
    /// Convert image to PNG format
    fn to_png(&self, img: &DynamicImage) -> Result<Bytes> {
        let mut buffer = Vec::new();
        let mut cursor = Cursor::new(&mut buffer);
        
        img.write_to(&mut cursor, ImageFormat::Png)
            .context("Failed to encode PNG")?;
        
        Ok(Bytes::from(buffer))
    }
}

/// Parse Accept header to determine preferred image format
/// 
/// # Example
/// For the Accept header: `image/avif,image/webp,image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5`
/// 
/// With default config (enable_avif=true, enable_webp=true):
/// - image/avif has quality 1.0 (default when not specified)
/// - image/webp has quality 1.0 (default when not specified)
/// - image/png has quality 1.0 (default when not specified)
/// - image/svg+xml is not supported (ignored)
/// - image/* has quality 0.8
/// - */* has quality 0.5
/// 
/// Result: Returns Avif (first format with highest quality 1.0)
pub fn parse_accept_header(accept: &str, enable_avif: bool, enable_webp: bool) -> OutputFormat {
    // Parse media types and their quality values
    let mut formats: Vec<(OutputFormat, f32)> = Vec::new();
    
    for part in accept.split(',') {
        let part = part.trim();
        let mut segments = part.split(';');
        let media_type = segments.next().unwrap_or("").trim();
        
        // Extract quality value (default to 1.0)
        let quality = segments
            .find_map(|s| {
                let s = s.trim();
                s.strip_prefix("q=")?.parse::<f32>().ok()
            })
            .unwrap_or(1.0);
        
        // Map media type to output format
        let format = match media_type {
            "image/avif" if enable_avif => Some(OutputFormat::Avif),
            "image/webp" if enable_webp => Some(OutputFormat::WebP),
            "image/jpeg" => Some(OutputFormat::Jpeg),
            "image/png" => Some(OutputFormat::Png),
            "image/*" => Some(OutputFormat::Original),
            "*/*" => Some(OutputFormat::Original),
            _ => None,
        };
        
        if let Some(fmt) = format {
            formats.push((fmt, quality));
        }
    }
    
    // Sort by quality (descending)
    // Use unwrap_or to handle NaN values gracefully (treat as equal)
    formats.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    
    // Return the highest quality format, or Original if none found
    formats.first()
        .map(|(fmt, _)| *fmt)
        .unwrap_or(OutputFormat::Original)
}

/// Check if content type is an image
pub fn is_image_content_type(content_type: &str) -> bool {
    content_type.starts_with("image/")
}

/// Get OutputFormat from content-type string
pub fn format_from_content_type(content_type: &str) -> Option<OutputFormat> {
    match content_type {
        "image/avif" => Some(OutputFormat::Avif),
        "image/webp" => Some(OutputFormat::WebP),
        "image/jpeg" | "image/jpg" => Some(OutputFormat::Jpeg),
        "image/png" => Some(OutputFormat::Png),
        _ => None,
    }
}

/// Check if the upstream format satisfies the desired format
/// Returns true if no conversion is needed
pub fn format_satisfies(upstream_format: OutputFormat, desired_format: OutputFormat) -> bool {
    upstream_format == desired_format || desired_format == OutputFormat::Original
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_accept_avif_preferred() {
        let accept = "image/avif,image/webp,image/jpeg";
        let format = parse_accept_header(accept, true, true);
        assert_eq!(format, OutputFormat::Avif);
    }

    #[test]
    fn test_parse_accept_webp_preferred() {
        let accept = "image/webp;q=1.0,image/avif;q=0.8,image/jpeg;q=0.5";
        let format = parse_accept_header(accept, true, true);
        assert_eq!(format, OutputFormat::WebP);
    }

    #[test]
    fn test_parse_accept_avif_disabled() {
        let accept = "image/avif,image/webp,image/jpeg";
        let format = parse_accept_header(accept, false, true);
        assert_eq!(format, OutputFormat::WebP);
    }

    #[test]
    fn test_is_image_content_type() {
        assert!(is_image_content_type("image/jpeg"));
        assert!(is_image_content_type("image/png"));
        assert!(!is_image_content_type("text/html"));
        assert!(!is_image_content_type("application/json"));
    }

    #[test]
    fn test_format_from_content_type() {
        assert_eq!(format_from_content_type("image/avif"), Some(OutputFormat::Avif));
        assert_eq!(format_from_content_type("image/webp"), Some(OutputFormat::WebP));
        assert_eq!(format_from_content_type("image/jpeg"), Some(OutputFormat::Jpeg));
        assert_eq!(format_from_content_type("image/jpg"), Some(OutputFormat::Jpeg));
        assert_eq!(format_from_content_type("image/png"), Some(OutputFormat::Png));
        assert_eq!(format_from_content_type("image/gif"), None);
        assert_eq!(format_from_content_type("text/plain"), None);
    }

    #[test]
    fn test_format_satisfies() {
        // Same format satisfies
        assert!(format_satisfies(OutputFormat::Avif, OutputFormat::Avif));
        assert!(format_satisfies(OutputFormat::WebP, OutputFormat::WebP));
        
        // Original always satisfied
        assert!(format_satisfies(OutputFormat::Avif, OutputFormat::Original));
        assert!(format_satisfies(OutputFormat::Jpeg, OutputFormat::Original));
        
        // Different formats don't satisfy
        assert!(!format_satisfies(OutputFormat::Jpeg, OutputFormat::Avif));
        assert!(!format_satisfies(OutputFormat::Png, OutputFormat::WebP));
    }
}
