//! Paired lamps, stored as one OS credential so addresses and tokens agree.
//! One lamp is active; the remote client talks only to the active lamp.
use keyring::{Entry, Error};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

static STORE_LOCK: Mutex<()> = Mutex::new(());
const SERVICE: &str = "org.orion.studio.pairing";
const ACCOUNT: &str = "paired-orion";

#[derive(Clone, Serialize, Deserialize)]
pub struct Pairing {
    pub url: String,
    pub token: String,
}

impl Pairing {
    pub fn validate(&self) -> Result<(), String> {
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

/// A saved lamp without its token, for listing in Studio.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct PairedLamp {
    pub url: String,
    pub active: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    active: Option<String>,
    lamps: Vec<Pairing>,
}

impl Store {
    fn active(&self) -> Option<&Pairing> {
        let active = self.active.as_deref()?;
        self.lamps.iter().find(|lamp| lamp.url == active)
    }
}

fn read(entry: &Entry) -> Result<Store, String> {
    let value = match entry.get_password() {
        Ok(value) => value,
        Err(Error::NoEntry) => return Ok(Store::default()),
        // Never format keyring errors: some variants contain secret bytes.
        Err(_) => {
            return Err("Could not read the system credential store. Unlock it and retry.".into());
        }
    };
    let invalid = "Saved pairing is invalid. Forget Orion and pair again.";
    // Studio saved a single pairing object before it supported several lamps.
    let store = match serde_json::from_str::<Store>(&value) {
        Ok(store) => store,
        Err(_) => {
            let pairing: Pairing = serde_json::from_str(&value).map_err(|_| invalid)?;
            Store {
                active: Some(pairing.url.clone()),
                lamps: vec![pairing],
            }
        }
    };
    for lamp in &store.lamps {
        lamp.validate()?;
    }
    if store.active.is_some() && store.active().is_none() {
        return Err(invalid.into());
    }
    Ok(store)
}

fn write(entry: &Entry, store: &Store) -> Result<(), String> {
    if store.lamps.is_empty() {
        return match entry.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(_) => {
                Err("Could not forget Orion. Unlock the system credential store and retry.".into())
            }
        };
    }
    let value = serde_json::to_string(store).map_err(|_| "Could not encode pairing.")?;
    entry.set_password(&value).map_err(|_| {
        "Could not save pairing in the system credential store. Unlock it and retry.".into()
    })
}

fn load(entry: &Entry) -> Result<Option<Pairing>, String> {
    Ok(read(entry)?.active().cloned())
}

/// Save or update a lamp by address and make it the active lamp.
fn save(entry: &Entry, pairing: &Pairing) -> Result<(), String> {
    pairing.validate()?;
    let mut store = read(entry)?;
    match store.lamps.iter_mut().find(|lamp| lamp.url == pairing.url) {
        Some(existing) => *existing = pairing.clone(),
        None => store.lamps.push(pairing.clone()),
    }
    store.active = Some(pairing.url.clone());
    write(entry, &store)
}

/// Forget the active lamp; the next saved lamp, if any, becomes active.
fn forget(entry: &Entry) -> Result<(), String> {
    let mut store = read(entry)?;
    if let Some(active) = store.active.take() {
        store.lamps.retain(|lamp| lamp.url != active);
    }
    store.active = store.lamps.first().map(|lamp| lamp.url.clone());
    write(entry, &store)
}

fn list(entry: &Entry) -> Result<Vec<PairedLamp>, String> {
    let store = read(entry)?;
    Ok(store
        .lamps
        .iter()
        .map(|lamp| PairedLamp {
            url: lamp.url.clone(),
            active: store.active.as_deref() == Some(lamp.url.as_str()),
        })
        .collect())
}

fn select(entry: &Entry, url: &str) -> Result<(), String> {
    let mut store = read(entry)?;
    if !store.lamps.iter().any(|lamp| lamp.url == url) {
        return Err("That lamp is not paired on this computer.".into());
    }
    store.active = Some(url.to_owned());
    write(entry, &store)
}

async fn with_store<T: Send + 'static>(
    operation: impl FnOnce(&Entry) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(move || {
        let _guard = STORE_LOCK
            .lock()
            .map_err(|_| "Credential store is unavailable.")?;
        let entry = Entry::new(SERVICE, ACCOUNT).map_err(|_| "Credential store is unavailable.")?;
        operation(&entry)
    })
    .await
    .map_err(|_| "Credential store operation failed.".to_owned())?
}

pub async fn load_pairing() -> Result<Option<Pairing>, String> {
    with_store(load).await
}

pub async fn save_pairing(pairing: Pairing) -> Result<(), String> {
    with_store(move |entry| save(entry, &pairing)).await
}

pub async fn forget_pairing() -> Result<(), String> {
    with_store(forget).await
}

pub async fn list_pairings() -> Result<Vec<PairedLamp>, String> {
    with_store(list).await
}

pub async fn select_pairing(url: String) -> Result<(), String> {
    with_store(move |entry| select(entry, &url)).await
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

    fn lamp(url: &str, token: char) -> Pairing {
        Pairing {
            url: url.into(),
            token: token.to_string().repeat(32),
        }
    }

    #[test]
    fn several_lamps_are_saved_switched_and_forgotten_one_at_a_time() {
        let entry = Entry::new_with_credential(Box::new(keyring::mock::MockCredential::default()));
        save(&entry, &lamp("http://orion.local:7447", 'a')).unwrap();
        save(&entry, &lamp("http://ariadne-robot.local:7447", 'b')).unwrap();
        assert_eq!(
            load(&entry).unwrap().unwrap().url,
            "http://ariadne-robot.local:7447"
        );
        assert_eq!(
            list(&entry).unwrap(),
            vec![
                PairedLamp {
                    url: "http://orion.local:7447".into(),
                    active: false
                },
                PairedLamp {
                    url: "http://ariadne-robot.local:7447".into(),
                    active: true
                },
            ]
        );
        select(&entry, "http://orion.local:7447").unwrap();
        assert_eq!(load(&entry).unwrap().unwrap().token, "a".repeat(32));
        assert!(select(&entry, "http://unknown.local:7447").is_err());
        // Re-pairing an address replaces its token instead of adding a duplicate.
        save(&entry, &lamp("http://orion.local:7447", 'c')).unwrap();
        assert_eq!(list(&entry).unwrap().len(), 2);
        forget(&entry).unwrap();
        assert_eq!(
            load(&entry).unwrap().unwrap().url,
            "http://ariadne-robot.local:7447"
        );
        forget(&entry).unwrap();
        assert!(load(&entry).unwrap().is_none());
        assert!(list(&entry).unwrap().is_empty());
    }

    #[test]
    fn a_single_pairing_saved_by_older_studio_still_loads() {
        let entry = Entry::new_with_credential(Box::new(keyring::mock::MockCredential::default()));
        let old = lamp("http://orion.local:7447", 'a');
        entry
            .set_password(&serde_json::to_string(&old).unwrap())
            .unwrap();
        assert_eq!(load(&entry).unwrap().unwrap().url, old.url);
        save(&entry, &lamp("http://ariadne-robot.local:7447", 'b')).unwrap();
        assert_eq!(list(&entry).unwrap().len(), 2);
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
