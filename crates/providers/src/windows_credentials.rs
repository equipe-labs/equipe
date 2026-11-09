//! Windows Credential Manager access shared by the daemon's secret stores.

use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};

fn wide(value: &str) -> io::Result<Vec<u16>> {
    if value.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a credential name cannot contain NUL",
        ));
    }
    Ok(value.encode_utf16().chain(std::iter::once(0)).collect())
}

fn target(service: &str, account: &str) -> io::Result<Vec<u16>> {
    wide(&format!("{service}:{account}"))
}

/// A stable account name scoped to one equipe home, so test and alternate
/// homes never read each other's credentials.
pub fn scoped_account(home: &Path, account: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(home.as_os_str().to_string_lossy().as_bytes());
    let digest = hash.finalize();
    format!(
        "{:02x}{:02x}{:02x}{:02x}:{account}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

/// Store opaque bytes as a generic credential for the current Windows user.
pub fn set(service: &str, account: &str, secret: &[u8]) -> io::Result<()> {
    let mut target = target(service, account)?;
    let mut username = wide(account)?;
    let mut blob = secret.to_vec();
    let credential = CREDENTIALW {
        Flags: 0,
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        Comment: std::ptr::null_mut(),
        LastWritten: Default::default(),
        CredentialBlobSize: blob
            .len()
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "credential is too large"))?,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: std::ptr::null_mut(),
        UserName: username.as_mut_ptr(),
    };
    // SAFETY: every pointer in `credential` refers to a live mutable buffer
    // for the duration of the call; Windows copies the credential.
    if unsafe { CredWriteW(&credential, 0) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Read opaque bytes from the current user's Credential Manager.
pub fn get(service: &str, account: &str) -> io::Result<Option<Vec<u8>>> {
    let target = target(service, account)?;
    let mut credential = std::ptr::null_mut();
    // SAFETY: `target` is NUL-terminated and `credential` is a valid out pointer.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    // SAFETY: CredReadW returned a valid allocation and blob. Copy it before
    // releasing the whole allocation with CredFree.
    let bytes = unsafe {
        let item = &*credential;
        let bytes =
            std::slice::from_raw_parts(item.CredentialBlob, item.CredentialBlobSize as usize)
                .to_vec();
        CredFree(credential.cast());
        bytes
    };
    Ok(Some(bytes))
}

/// Delete a credential, reporting whether one existed.
pub fn delete(service: &str, account: &str) -> io::Result<bool> {
    let target = target(service, account)?;
    // SAFETY: `target` is NUL-terminated and live for the call.
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_survives_a_fresh_reader() {
        let account = format!("test-{}", uuid::Uuid::new_v4());
        set("equipe.test", &account, b"secret after restart").unwrap();
        assert_eq!(
            get("equipe.test", &account).unwrap().as_deref(),
            Some(b"secret after restart".as_slice())
        );
        assert!(delete("equipe.test", &account).unwrap());
        assert_eq!(get("equipe.test", &account).unwrap(), None);
    }
}
