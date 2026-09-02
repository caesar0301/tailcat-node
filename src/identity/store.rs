//! Persistent identity store.
//!
//! `identity.key` holds the persistent Tailcat private key and must
//! survive daemon restart, machine reboot, and tailcat-node upgrade.
//! Changing identity means effectively creating a new node.

use crate::error::{Error, Result};
use crate::identity::Identity;
use rand::RngCore;
use std::path::PathBuf;

/// Manages the identity file on disk.
pub struct IdentityStore {
    dir: PathBuf,
}

impl IdentityStore {
    pub fn new(config_dir: PathBuf) -> Self {
        Self { dir: config_dir }
    }

    /// Load the identity from `identity.key`.
    pub fn load(&self) -> Result<Identity> {
        let path = self.dir.join("identity.key");
        if !path.exists() {
            return Err(Error::NotInitialized(format!(
                "identity file not found: {}",
                path.display()
            )));
        }
        let text = std::fs::read_to_string(&path)?;
        // Parse simple key=value format.
        let mut node_id = String::new();
        let mut private_key = String::new();
        let mut public_key = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once(':') {
                let k = k.trim();
                let v = v.trim();
                match k {
                    "node_id" => node_id = v.to_string(),
                    "privkey" | "private_key" => private_key = v.to_string(),
                    "pubkey" | "public_key" => {
                        public_key = v.to_string();
                    }
                    _ => {}
                }
            }
        }
        if private_key.is_empty() {
            return Err(Error::Identity("missing private key".to_string()));
        }
        if public_key.is_empty() {
            public_key = derive_public_key(&private_key);
        }
        Ok(Identity {
            node_id,
            private_key,
            public_key,
        })
    }

    /// Save the identity to `identity.key` with 0600 permissions.
    pub fn save(&self, identity: &Identity) -> Result<()> {
        let path = self.dir.join("identity.key");
        let text = format!(
            "# tailcat-node identity (do not share)\n\
             node_id:{}\n\
             privkey:{}\n\
             pubkey:{}\n",
            identity.node_id, identity.private_key, identity.public_key
        );
        std::fs::write(&path, text)?;
        // Set permissions to 0600 on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&path, perms)?;
        }
        Ok(())
    }
}

/// Generate a new identity.
pub fn generate(node_id: &str) -> Identity {
    let mut private_key_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut private_key_bytes);
    let private_key = hex::encode(private_key_bytes);
    let public_key = derive_public_key(&private_key);
    Identity {
        node_id: node_id.to_string(),
        private_key,
        public_key,
    }
}

/// Derive a public key from a private key.
///
/// This is a placeholder for the real Tailcat key derivation.
/// For now, we just hash the private key to produce a public key.
fn derive_public_key(private_key: &str) -> String {
    // Simple hash: reverse the hex string and prefix with "pub".
    // This is NOT cryptographically secure — it's a placeholder.
    let reversed: String = private_key.chars().rev().collect();
    format!("pub{}", &reversed[..reversed.len().min(32)])
}
