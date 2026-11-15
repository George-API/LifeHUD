//! Secrets (AI provider API keys) in Windows Credential Manager. Windows encrypts them for the
//! current user; they never touch `settings.json`, the log, or any other file Life HUD writes.

use windows::Win32::Security::Credentials::{
    CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
    CredReadW, CredWriteW,
};
use windows::core::{HSTRING, PWSTR};

/// Reads a secret, or `None` if none is stored (or it isn't valid UTF-8).
pub fn read(target: &str) -> Option<String> {
    let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        CredReadW(&HSTRING::from(target), CRED_TYPE_GENERIC, None, &mut cred).ok()?;
        // SAFETY: on success `cred` points to a CREDENTIALW the system owns until CredFree. The blob
        // pointer is only read when non-null, for exactly the size the system reports.
        let c = &*cred;
        let secret = if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
            None
        } else {
            let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
            String::from_utf8(blob.to_vec()).ok()
        };
        CredFree(cred as *const _);
        secret
    }
}

/// Stores (or replaces) a secret. Returns false if Windows refused it or it's too large.
pub fn write(target: &str, secret: &str) -> bool {
    if secret.is_empty() || secret.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
        return false;
    }
    let mut target_w = super::to_wide(target);
    let mut user_w = super::to_wide("Life HUD");
    let mut blob = secret.as_bytes().to_vec();
    let cred = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target_w.as_mut_ptr()),
        UserName: PWSTR(user_w.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        // Per user, on this PC only: never roams to other machines with the profile.
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // SAFETY: every pointer in `cred` refers to a live local buffer for the duration of the call.
    let ok = unsafe { CredWriteW(&cred, 0).is_ok() };
    // Don't leave a plaintext copy in freed heap memory.
    blob.iter_mut().for_each(|b| *b = 0);
    ok
}

pub fn delete(target: &str) {
    unsafe {
        let _ = CredDeleteW(&HSTRING::from(target), CRED_TYPE_GENERIC, None);
    }
}

pub fn exists(target: &str) -> bool {
    read(target).is_some()
}
