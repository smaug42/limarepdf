//! Password protection (RC4-40, RC4-128, AES-128, AES-256) applied when a document is written.

use crate::{Pdf, Result};
use lopdf::encryption::crypt_filters::{Aes128CryptFilter, Aes256CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, Permissions};
use rand::Rng;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cipher {
    Rc4_40,
    Rc4_128,
    Aes128,
    Aes256,
}

/// Which actions remain allowed for users who open the file with the user password.
#[derive(Clone, Copy, Debug)]
pub struct Restrictions {
    pub print: bool,
    pub print_high_quality: bool,
    pub modify: bool,
    pub copy: bool,
    pub annotate: bool,
    pub fill_forms: bool,
    pub assemble: bool,
}

impl Default for Restrictions {
    fn default() -> Self {
        Self {
            print: true,
            print_high_quality: true,
            modify: true,
            copy: true,
            annotate: true,
            fill_forms: true,
            assemble: true,
        }
    }
}

impl Restrictions {
    fn flags(&self) -> Permissions {
        let mut p = Permissions::empty();
        p.set(Permissions::PRINTABLE, self.print);
        p.set(
            Permissions::PRINTABLE_IN_HIGH_QUALITY,
            self.print && self.print_high_quality,
        );
        p.set(Permissions::MODIFIABLE, self.modify);
        p.set(Permissions::COPYABLE, self.copy);
        p.set(Permissions::ANNOTABLE, self.annotate);
        p.set(Permissions::FILLABLE, self.fill_forms);
        p.set(Permissions::ASSEMBLABLE, self.assemble);
        p
    }
}

#[derive(Clone, Debug)]
pub struct Encryption {
    pub user_password: String,
    pub owner_password: String,
    pub cipher: Cipher,
    pub restrictions: Restrictions,
}

impl Encryption {
    pub(crate) fn state(&self, doc: &Document) -> Result<EncryptionState> {
        let perms = self.restrictions.flags();
        let (u, o) = (self.user_password.as_str(), self.owner_password.as_str());
        let version = match self.cipher {
            Cipher::Rc4_40 => EncryptionVersion::V1 {
                document: doc,
                owner_password: o,
                user_password: u,
                permissions: perms,
            },
            Cipher::Rc4_128 => EncryptionVersion::V2 {
                document: doc,
                owner_password: o,
                user_password: u,
                key_length: 128,
                permissions: perms,
            },
            Cipher::Aes128 => {
                let f: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
                return Ok(EncryptionState::try_from(EncryptionVersion::V4 {
                    document: doc,
                    encrypt_metadata: true,
                    crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), f)]),
                    stream_filter: b"StdCF".to_vec(),
                    string_filter: b"StdCF".to_vec(),
                    owner_password: o,
                    user_password: u,
                    permissions: perms,
                })?);
            }
            Cipher::Aes256 => {
                let f: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
                let mut key = [0u8; 32];
                rand::rng().fill(&mut key);
                return Ok(EncryptionState::try_from(EncryptionVersion::V5 {
                    encrypt_metadata: true,
                    crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), f)]),
                    file_encryption_key: &key,
                    stream_filter: b"StdCF".to_vec(),
                    string_filter: b"StdCF".to_vec(),
                    owner_password: o,
                    user_password: u,
                    permissions: perms,
                })?);
            }
        };
        Ok(EncryptionState::try_from(version)?)
    }
}

impl Pdf {
    /// Protect the document with passwords when it is next saved (`None` removes protection).
    pub fn set_encryption(&mut self, enc: Option<Encryption>) {
        self.encryption = enc;
    }
}
