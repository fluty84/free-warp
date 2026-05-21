//! Implementations of the [`SecureStorage`] service for the macOS platform.

use std::path::PathBuf;

use anyhow::{anyhow, Context};
use rand::RngCore;
use ring::aead;
use security_framework::os::macos::{
    keychain::SecKeychain, keychain_item::SecKeychainItem, passwords::SecKeychainItemPassword,
};

use super::Error;

/// Implementation of the SecureStorage service using macOS Security
/// framework keychains, with an optional file-based fallback.
///
/// When `fallback_dir` is set, all reads/writes go to encrypted files in that
/// directory instead of the Keychain — useful for dev builds where the binary
/// changes on every `cargo build` and Keychain would prompt on every run.
pub struct SecureStorage {
    /// The name of the service under which to store the values.
    service_name: String,

    /// When set, skip the Keychain and store in encrypted files here.
    fallback_dir: Option<PathBuf>,

    encryption_key: std::cell::OnceCell<Option<aead::LessSafeKey>>,
}

impl SecureStorage {
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_owned(),
            fallback_dir: None,
            encryption_key: Default::default(),
        }
    }

    /// Creates a variant that stores everything in encrypted files under
    /// `dir` instead of the macOS Keychain.  Avoids repeated prompts in
    /// dev builds where the binary identity changes on every compile.
    pub fn new_with_dir(service_name: &str, dir: PathBuf) -> Self {
        Self {
            service_name: service_name.to_owned(),
            fallback_dir: Some(dir),
            encryption_key: Default::default(),
        }
    }
}

// ── Keychain helpers ─────────────────────────────────────────────────────────

impl SecureStorage {
    fn get_password_item(
        &self,
        key: &str,
    ) -> Result<(SecKeychainItemPassword, SecKeychainItem), Error> {
        let keychain = SecKeychain::default()?;
        keychain
            .find_generic_password(&self.service_name, key)
            .map_err(|_| Error::NotFound)
    }
}

// ── File-based fallback helpers ───────────────────────────────────────────────

impl SecureStorage {
    fn encryption_key(&self) -> Result<&aead::LessSafeKey, Error> {
        self.encryption_key
            .get_or_init(|| {
                let mut key_bytes =
                    Vec::from("https://releases.warp.dev/channel_versions.json");
                key_bytes.resize(aead::AES_256_GCM.key_len(), 0);
                match aead::UnboundKey::new(&aead::AES_256_GCM, key_bytes.as_slice()) {
                    Ok(k) => Some(aead::LessSafeKey::new(k)),
                    Err(_) => {
                        log::error!("Failed to initialize fallback encryption key");
                        None
                    }
                }
            })
            .as_ref()
            .ok_or_else(|| Error::Unknown(anyhow!("Invalid encryption key")))
    }

    fn fallback_file(&self, key: &str) -> Result<PathBuf, Error> {
        let dir = self.fallback_dir.as_ref().ok_or(Error::NotFound)?;
        Ok(dir.join(format!("{}-{key}", self.service_name)))
    }

    fn encrypt(&self, value: &str) -> Result<Vec<u8>, Error> {
        let key = self.encryption_key()?;
        let mut nonce_bytes = [0u8; aead::NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
        let mut data = value.as_bytes().to_vec();
        key.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut data)
            .map_err(Into::<Error>::into)
            .context("Fallback encryption failed")?;
        let mut out = Vec::with_capacity(aead::NONCE_LEN + data.len());
        out.extend_from_slice(&nonce_bytes);
        out.append(&mut data);
        Ok(out)
    }

    fn decrypt(&self, value: &[u8]) -> Result<String, Error> {
        if value.len() < aead::NONCE_LEN + 1 {
            return Err(Error::Unknown(anyhow!("Ciphertext too short")));
        }
        let key = self.encryption_key()?;
        let nonce = aead::Nonce::try_assume_unique_for_key(&value[..aead::NONCE_LEN])
            .map_err(Into::<Error>::into)
            .context("Failed to parse nonce")?;
        let mut data = value[aead::NONCE_LEN..].to_owned();
        let len = key
            .open_in_place(nonce, aead::Aad::empty(), &mut data)
            .map_err(Into::<Error>::into)
            .context("Fallback decryption failed")?
            .len();
        data.resize(len, 0);
        String::from_utf8(data).map_err(|e| Error::DecodeError(e.utf8_error()))
    }

    fn write_file(&self, key: &str, value: &str) -> Result<(), Error> {
        let path = self.fallback_file(key)?;
        let encrypted = self.encrypt(value)?;
        std::fs::write(&path, encrypted).map_err(|e| Error::Unknown(e.into()))?;
        // Restrict permissions to owner-only (rw-------).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    fn read_file(&self, key: &str) -> Result<String, Error> {
        let path = self.fallback_file(key)?;
        let data = std::fs::read(path).map_err(|_| Error::NotFound)?;
        self.decrypt(&data)
    }

    fn remove_file(&self, key: &str) -> Result<(), Error> {
        let path = self.fallback_file(key)?;
        std::fs::remove_file(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound,
            _ => Error::Unknown(e.into()),
        })
    }
}

// ── SecureStorage trait impl ──────────────────────────────────────────────────

impl super::SecureStorage for SecureStorage {
    fn write_value(&self, key: &str, value: &str) -> Result<(), Error> {
        if self.fallback_dir.is_some() {
            return self.write_file(key, value);
        }
        let keychain = SecKeychain::default()?;
        keychain
            .set_generic_password(self.service_name.as_str(), key, value.as_bytes())
            .map_err(Into::into)
    }

    fn read_value(&self, key: &str) -> Result<String, Error> {
        if self.fallback_dir.is_some() {
            return self.read_file(key);
        }
        let (password, _) = self.get_password_item(key)?;
        String::from_utf8(password.as_ref().to_vec())
            .map_err(|err| Error::DecodeError(err.utf8_error()))
    }

    fn remove_value(&self, key: &str) -> Result<(), Error> {
        if self.fallback_dir.is_some() {
            return self.remove_file(key);
        }
        let (_, item) = self.get_password_item(key)?;
        item.delete();
        Ok(())
    }
}

// ── Error conversions ─────────────────────────────────────────────────────────

impl From<security_framework::base::Error> for Error {
    fn from(value: security_framework::base::Error) -> Self {
        Error::Unknown(anyhow!(value))
    }
}

impl From<ring::error::Unspecified> for Error {
    fn from(value: ring::error::Unspecified) -> Self {
        Error::Unknown(anyhow!(value))
    }
}

