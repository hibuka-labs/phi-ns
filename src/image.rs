//! Multimodal returns (sdk-assembly §4.2).
//!
//! A screenshot is something the *model looks at*, not text. The channel is
//! `CellEvent::Image(ImageRef)` on the observation stream — no per-pack
//! image inventing, no image bytes in `stdout`.
//!
//! Three rules:
//!
//! 1. images travel as [`ImageRef`] events, never in `stdout`
//! 2. default is on-disk `Path`; `Inline` must be requested explicitly
//! 3. `alt` is mandatory — when an image is trimmed by the event budget the
//!    model must still read "there was a picture of X" in the text stream

use crate::error::NsError;
use base64::Engine;
use std::path::PathBuf;

/// Where an image's bytes live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    /// Bytes carried inline (opt-in; counts against the event budget).
    Inline(Vec<u8>),
    /// On-disk path (default).
    Path(PathBuf),
}

/// A reference to an image handed to the model through the observation
/// stream. The worker-side handle (`ImageHandle`) and this reference point at
/// the same bytes — one source, two channels (code vs. model).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// MIME type, e.g. `"image/png"`.
    pub mime: &'static str,
    /// Where the bytes live.
    pub source: ImageSource,
    /// One sentence describing the image. **Required** — it is what the model
    /// reads when the image itself is dropped by limits.
    pub alt: String,
}

impl ImageRef {
    /// Reference an on-disk image (the default channel).
    pub fn from_path(mime: &'static str, path: impl Into<PathBuf>, alt: impl Into<String>) -> Self {
        Self {
            mime,
            source: ImageSource::Path(path.into()),
            alt: alt.into(),
        }
    }

    /// Carry the bytes inline — only for explicitly requested captures.
    pub fn inline(mime: &'static str, bytes: Vec<u8>, alt: impl Into<String>) -> Self {
        Self {
            mime,
            source: ImageSource::Inline(bytes),
            alt: alt.into(),
        }
    }

    /// Enforce the mandatory-`alt` rule at the choke point.
    pub fn validate(&self) -> Result<(), NsError> {
        if self.alt.trim().is_empty() {
            return Err(NsError::Contract(
                "ImageRef.alt is mandatory (rules for multimodal returns)".into(),
            ));
        }
        Ok(())
    }

    /// Wire form — **the** contract between a multimodal method's return
    /// value and the engine's event harvest. Both sides speak this shape and
    /// nothing else, so the channel has one home:
    ///
    /// ```json
    /// {"path": "/tmp/x.png", "mime": "image/png", "alt": "..."}
    /// {"inline_base64": "iVBOR…", "mime": "image/png", "alt": "..."}
    /// ```
    pub fn to_wire(&self) -> serde_json::Value {
        let mut out = serde_json::Map::new();
        out.insert(
            "mime".into(),
            serde_json::Value::String(self.mime.to_string()),
        );
        out.insert("alt".into(), serde_json::Value::String(self.alt.clone()));
        match &self.source {
            ImageSource::Path(p) => {
                out.insert(
                    "path".into(),
                    serde_json::Value::String(p.display().to_string()),
                );
            }
            ImageSource::Inline(bytes) => {
                out.insert(
                    "inline_base64".into(),
                    serde_json::Value::String(
                        base64::engine::general_purpose::STANDARD.encode(bytes),
                    ),
                );
            }
        }
        serde_json::Value::Object(out)
    }

    /// Parse wire images back into references.
    ///
    /// This is the engine-side choke point for multimodal returns: what the
    /// declaration said would happen (`MethodSpec.multimodal`) happens here,
    /// not by convention in each pack. Three shapes are accepted:
    ///
    /// * one image object — `{"path": …, "mime": …, "alt": …}`
    /// * an array of image objects
    /// * a **composite** object with an `"images"` array — a capture that
    ///   returns data *and* pictures (e.g. `m.observe(mode="vision")` returns
    ///   regions + one screenshot) keeps its data fields for the worker while
    ///   the harvest takes the images (P3).
    pub fn from_wire(value: &serde_json::Value) -> Result<Vec<ImageRef>, NsError> {
        if let Some(images) = value.get("images").and_then(serde_json::Value::as_array) {
            return images.iter().map(ImageRef::one_from_wire).collect();
        }
        let items: Vec<&serde_json::Value> = match value {
            serde_json::Value::Array(xs) => xs.iter().collect(),
            one => vec![one],
        };
        items.into_iter().map(ImageRef::one_from_wire).collect()
    }

    fn one_from_wire(value: &serde_json::Value) -> Result<ImageRef, NsError> {
        let obj = value.as_object().ok_or_else(|| {
            NsError::Contract("multimodal return must be an image object or a list of them".into())
        })?;
        let mime_wire = obj
            .get("mime")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| NsError::Contract("image wire value lacks `mime`".into()))?;
        let mime = static_mime(mime_wire)?;
        let alt = obj
            .get("alt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let source = if let Some(path) = obj.get("path").and_then(serde_json::Value::as_str) {
            ImageSource::Path(PathBuf::from(path))
        } else if let Some(b64) = obj.get("inline_base64").and_then(serde_json::Value::as_str) {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| NsError::Contract(format!("inline image is not valid base64: {e}")))?;
            ImageSource::Inline(bytes)
        } else {
            return Err(NsError::Contract(
                "image wire value needs `path` or `inline_base64`".into(),
            ));
        };
        let image = ImageRef { mime, source, alt };
        image.validate()?;
        Ok(image)
    }
}

/// MIME types the wire codec round-trips — closed on purpose: `ImageRef.mime`
/// is `&'static str`, so an unknown label cannot be adopted without leaking.
/// Fail closed and make the pack use one of these.
fn static_mime(mime: &str) -> Result<&'static str, NsError> {
    match mime {
        "image/png" => Ok("image/png"),
        "image/jpeg" => Ok("image/jpeg"),
        "image/webp" => Ok("image/webp"),
        "image/gif" => Ok("image/gif"),
        "image/bmp" => Ok("image/bmp"),
        other => Err(NsError::Contract(format!(
            "unsupported image mime {other:?}; use png/jpeg/webp/gif/bmp"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape is the contract — round-trip both channels and prove
    /// the mandatory-`alt` rule bites at the parse choke point.
    #[test]
    fn wire_round_trip_and_alt_rule() {
        let path_img = ImageRef::from_path("image/png", "/tmp/a.png", "the toolbar");
        let wire = path_img.to_wire();
        assert_eq!(ImageRef::from_wire(&wire).unwrap(), vec![path_img]);

        let inline_img = ImageRef::inline("image/png", vec![1, 2, 3], "a pixel");
        let wire = inline_img.to_wire();
        assert_eq!(ImageRef::from_wire(&wire).unwrap(), vec![inline_img]);

        let list = serde_json::json!([
            ImageRef::from_path("image/png", "/tmp/a.png", "one").to_wire(),
            ImageRef::from_path("image/jpeg", "/tmp/b.jpg", "two").to_wire(),
        ]);
        assert_eq!(ImageRef::from_wire(&list).unwrap().len(), 2);

        let no_alt = serde_json::json!({"path": "/tmp/a.png", "mime": "image/png", "alt": "  "});
        let err = ImageRef::from_wire(&no_alt).unwrap_err();
        assert!(err.to_string().contains("alt"), "{err}");
    }
}
