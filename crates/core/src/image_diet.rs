//! Pictures in a tool's answer, kept to what a member's conversation can
//! carry.
//!
//! A picture in a tool answer stays in the member's conversation and is
//! read again with every model call after it. A design library's search
//! answered with eight to a dozen screens a call, at full size. So an outside
//! MCP server's answer passes through [`tool_reply`] on its way to the
//! member ([`crate::gateway`]): at most [`MAX_IMAGES`] pictures, each no
//! longer than [`MAX_EDGE`] on its long side, as JPEG — the rest left out
//! with a line saying how many and how to see them.

use base64::Engine as _;
use serde_json::{Value, json};

/// The most pictures one tool answer carries.
pub const MAX_IMAGES: usize = 4;

/// The longest side of a picture in a tool answer, in pixels: enough to
/// judge a screen's layout and type.
pub const MAX_EDGE: u32 = 1280;

/// JPEG quality of a picture made smaller.
const QUALITY: u8 = 80;

/// Whether a JSON-RPC message from a server is a tool answer with pictures.
pub fn has_images(message: &Value) -> bool {
    message["result"]["content"].as_array().is_some_and(|items| items.iter().any(is_image))
}

fn is_image(item: &Value) -> bool {
    item["type"] == "image" && item["data"].is_string()
}

/// `message` with its pictures kept to [`MAX_IMAGES`], each made no longer
/// than [`MAX_EDGE`] (as JPEG, when that is smaller); the pictures left out
/// said in a line. Any other message is left as it is. Blocking: decoding
/// and encoding pictures takes a moment.
pub fn tool_reply(mut message: Value) -> Value {
    // `get_mut`, not indexing: a message without them is left without them.
    let Some(items) = message.get_mut("result").and_then(|r| r.get_mut("content")).and_then(Value::as_array_mut) else { return message };
    let total = items.iter().filter(|i| is_image(i)).count();
    if total == 0 {
        return message;
    }
    let mut kept = 0;
    let mut out = Vec::with_capacity(items.len());
    for mut item in std::mem::take(items) {
        if !is_image(&item) {
            out.push(item);
            continue;
        }
        if kept == MAX_IMAGES {
            continue;
        }
        kept += 1;
        if let Some((data, mime)) = item["data"].as_str().and_then(|d| smaller(d)) {
            item["data"] = Value::String(data);
            item["mimeType"] = Value::String(mime.to_owned());
        }
        out.push(item);
    }
    if total > kept {
        out.push(json!({
            "type": "text",
            "text": format!(
                "[equipe: {} more picture(s) of this answer left out — {kept} of {total} shown, each kept in your conversation. Ask for fewer results, or the next page, to see others.]",
                total - kept
            ),
        }));
    }
    *items = out;
    message
}

/// A picture (base64) no longer than [`MAX_EDGE`] on its long side, as
/// JPEG — `None` when it is small enough already, can't be read, or would
/// come out no smaller.
fn smaller(data: &str) -> Option<(String, &'static str)> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()?;
    let picture = image::load_from_memory(&bytes).ok()?;
    if picture.width().max(picture.height()) <= MAX_EDGE {
        return None;
    }
    let fitted = picture.resize(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Triangle).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, QUALITY).encode_image(&fitted).ok()?;
    (out.len() < bytes.len()).then(|| (base64::engine::general_purpose::STANDARD.encode(out), "image/jpeg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> String {
        // A gradient, so it doesn't compress to nothing.
        let picture = image::RgbImage::from_fn(width, height, |x, y| image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x * y) % 239) as u8]));
        let mut out = std::io::Cursor::new(Vec::new());
        picture.write_to(&mut out, image::ImageFormat::Png).unwrap();
        base64::engine::general_purpose::STANDARD.encode(out.into_inner())
    }

    fn reply(images: usize, width: u32) -> Value {
        let mut content = vec![json!({ "type": "text", "text": "12 screens found" })];
        let data = png(width, width / 2);
        for _ in 0..images {
            content.push(json!({ "type": "image", "data": data, "mimeType": "image/png" }));
        }
        json!({ "jsonrpc": "2.0", "id": 4, "result": { "content": content } })
    }

    fn size(data: &Value) -> (u32, u32) {
        let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_str().unwrap()).unwrap();
        let picture = image::load_from_memory(&bytes).unwrap();
        (picture.width(), picture.height())
    }

    #[test]
    fn a_search_answering_with_a_dozen_big_screens_keeps_four_made_smaller_and_says_so() {
        let before = reply(12, 2400);
        assert!(has_images(&before));
        let after = tool_reply(before.clone());
        let items = after["result"]["content"].as_array().unwrap();
        let images: Vec<&Value> = items.iter().filter(|i| i["type"] == "image").collect();
        assert_eq!(images.len(), MAX_IMAGES);
        for image in &images {
            assert_eq!(image["mimeType"], "image/jpeg");
            assert_eq!(size(&image["data"]), (MAX_EDGE, MAX_EDGE / 2));
        }
        assert_eq!(items[0]["text"], "12 screens found", "the words first, as they were");
        let said = items.last().unwrap()["text"].as_str().unwrap();
        assert!(said.contains("8 more picture(s)") && said.contains("4 of 12 shown") && said.contains("fewer results"), "{said}");
        assert!(after.to_string().len() * 4 < before.to_string().len(), "{} → {}", before.to_string().len(), after.to_string().len());
    }

    #[test]
    fn small_pictures_and_answers_without_pictures_go_through_untouched() {
        let small = reply(3, 640);
        assert_eq!(tool_reply(small.clone()), small);
        let words = json!({ "jsonrpc": "2.0", "id": 1, "result": { "content": [{ "type": "text", "text": "ok" }] } });
        assert!(!has_images(&words));
        assert_eq!(tool_reply(words.clone()), words);
        let listing = json!({ "jsonrpc": "2.0", "id": 2, "result": { "tools": [] } });
        assert_eq!(tool_reply(listing.clone()), listing);
        // A picture that can't be read is passed on as it came.
        let odd = json!({ "jsonrpc": "2.0", "id": 3, "result": { "content": [{ "type": "image", "data": "bm90IGFuIGltYWdl", "mimeType": "image/png" }] } });
        assert_eq!(tool_reply(odd.clone()), odd);
    }
}
