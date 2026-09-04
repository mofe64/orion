//! One paired robot, stored as a single OS credential so address and token agree.
use keyring::{Entry, Error};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

static STORE_LOCK: Mutex<()> = Mutex::new(());
const SERVICE: &str = "org.orion.studio.pairing";
const ACCOUNT: &str = "paired-orion";

#[derive(Clone, Serialize, Deserialize)]
pub struct Pairing {
    url: String,
    token: String,
}

impl Pairing {
    fn validate(&self) -> Result<(), String> {
        let url = url::Url::parse(&self.url).map_err(|_| "Enter a valid Orion address.")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err("Use an HTTP gateway address without credentials or a path.".into());
        }
        if self.token.trim().len() < 32 || self.token.len() > 4096 {
            return Err("Enter Orion's complete pairing token.".into());
        }
        Ok(())
    }
}

fn load(entry: &Entry) -> Result<Option<Pairing>, String> {
    match entry.get_password() {
        Ok(value) => {
            let pairing: Pairing = serde_json::from_str(&value)
                .map_err(|_| "Saved pairing is invalid. Forget Orion and pair again.")?;
            pairing.validate()?;
            Ok(Some(pairing))
        }
        Err(Error::NoEntry) => Ok(None),
        // Never format keyring errors: some variants contain secret bytes.
        Err(_) => Err("Could not read the system credential store. Unlock it and retry.".into()),
    }
}

fn save(entry: &Entry, pairing: &Pairing) -> Result<(), String> {
    pairing.validate()?;
    let value = serde_json::to_string(pairing).map_err(|_| "Could not encode pairing.")?;
    entry.set_password(&value).map_err(|_| {
        "Could not save pairing in the system credential store. Unlock it and retry.".into()
    })
}

fn forget(entry: &Entry) -> Result<(), String> {
    match entry.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(_) => {
            Err("Could not forget Orion. Unlock the system credential store and retry.".into())
        }
    }
}

async fn with_store<T: Send + 'static>(
    operation: impl FnOnce(&Entry) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = STORE_LOCK
            .lock()
            .map_err(|_| "Credential store is unavailable.")?;
        let entry = Entry::new(SERVICE, ACCOUNT).map_err(|_| "Credential store is unavailable.")?;
        operation(&entry)
    })
    .await
    .map_err(|_| "Credential store operation failed.".to_owned())?
}

#[tauri::command]
pub async fn load_pairing() -> Result<Option<Pairing>, String> {
    with_store(load).await
}

#[tauri::command]
pub async fn save_pairing(pairing: Pairing) -> Result<(), String> {
    with_store(move |entry| save(entry, &pairing)).await
}

#[tauri::command]
pub async fn forget_pairing() -> Result<(), String> {
    with_store(forget).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_round_trip_and_idempotent_forget() {
        let entry = Entry::new_with_credential(Box::new(keyring::mock::MockCredential::default()));
        assert!(load(&entry).unwrap().is_none());
        let pairing = Pairing {
            url: "http://orion.local:7447".into(),
            token: "a".repeat(32),
        };
        save(&entry, &pairing).unwrap();
        let restored = load(&entry).unwrap().unwrap();
        assert_eq!(restored.url, pairing.url);
        assert_eq!(restored.token, pairing.token);
        forget(&entry).unwrap();
        forget(&entry).unwrap();
        assert!(load(&entry).unwrap().is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Touches a disposable entry in the macOS credential store"]
    fn native_keychain_round_trip() {
        let account = format!("test-{}", uuid::Uuid::new_v4());
        let entry = Entry::new(SERVICE, &account).unwrap();
        let pairing = Pairing {
            url: "http://127.0.0.1:7447".into(),
            token: "test-only-credential-".repeat(3),
        };
        save(&entry, &pairing).unwrap();
        // A fresh handle verifies persistence beyond the Entry instance.
        let restored = load(&Entry::new(SERVICE, &account).unwrap());
        forget(&entry).unwrap();
        assert_eq!(restored.unwrap().unwrap().token, pairing.token);
        assert!(load(&entry).unwrap().is_none());
    }

    #[test]
    fn invalid_pairing_does_not_replace_saved_credential() {
        let entry = Entry::new_with_credential(Box::new(keyring::mock::MockCredential::default()));
        let mut pairing = Pairing {
            url: "http://orion.local:7447".into(),
            token: "a".repeat(32),
        };
        save(&entry, &pairing).unwrap();
        for url in [
            "file:///tmp/a",
            "http://user:secret@orion.local",
            "http://orion.local?token=x",
            "http://orion.local/api",
        ] {
            pairing.url = url.into();
            assert!(save(&entry, &pairing).is_err());
        }
        assert_eq!(
            load(&entry).unwrap().unwrap().url,
            "http://orion.local:7447"
        );
    }
}
