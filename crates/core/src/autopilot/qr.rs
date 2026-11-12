//! An authenticator QR code read off a site's two-factor setup page, here on
//! this machine: the browser host takes a picture of the person's own tab on
//! the login's site, and equipe looks for an `otpauth://totp/` code in it.
//! The picture goes nowhere else and is dropped once read.

/// The `otpauth://totp/…` address in a picture (PNG or JPEG), if any.
pub fn otpauth_in(picture: &[u8]) -> Option<String> {
    let image = image::load_from_memory(picture).ok()?.to_luma8();
    let (w, h) = (image.width() as usize, image.height() as usize);
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| image.get_pixel(x as u32, y as u32).0[0]);
    prepared.detect_grids().into_iter().filter_map(|grid| grid.decode().ok().map(|(_, text)| text)).find(|text| text.to_ascii_lowercase().starts_with("otpauth://totp/"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_authenticator_address_is_read_off_a_picture_of_the_setup_page() {
        let found = super::otpauth_in(include_bytes!("testdata/totp-qr.png")).unwrap();
        assert!(found.starts_with("otpauth://totp/Example:abel?secret=GEZDGNBVGY3TQOJQ"), "{found}");
        assert_eq!(super::super::totp::Totp::parse(&found).unwrap().code_at(59), "287082");
        assert_eq!(super::otpauth_in(b"not a picture"), None);
    }
}
