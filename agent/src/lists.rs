//! Short-lived lists are separate from personal memories and model instructions.
use crate::{
    AgentConfig,
    memory::{self, Entry},
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};
pub type Item = Entry;
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ListName {
    Shopping,
    Todo,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Lists {
    shopping: Vec<Item>,
    todo: Vec<Item>,
}
impl Lists {
    fn items(&self, list: ListName) -> &Vec<Item> {
        match list {
            ListName::Shopping => &self.shopping,
            ListName::Todo => &self.todo,
        }
    }
    fn items_mut(&mut self, list: ListName) -> &mut Vec<Item> {
        match list {
            ListName::Shopping => &mut self.shopping,
            ListName::Todo => &mut self.todo,
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ListChange {
    Add {
        text: String,
    },
    Remove {
        expected: Item,
    },
    Clear {
        expected: Vec<Item>,
        confirmed: bool,
    },
}
fn validate(text: &str) -> Result<(), String> {
    if !(1..=200).contains(&text.trim().chars().count()) || text.chars().any(char::is_control) {
        return Err("List items need 1–200 printable characters".into());
    }
    Ok(())
}
fn read(path: &Path) -> Result<Lists, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Lists::default()),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = vec![];
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Lists file is too large".into());
    }
    let lists: Lists = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut ids = std::collections::HashSet::new();
    for items in [&lists.shopping, &lists.todo] {
        if items.len() > 200 {
            return Err("At most 200 items per list".into());
        }
        let mut texts = std::collections::HashSet::new();
        for item in items {
            validate(&item.text)?;
            if item.id.is_empty()
                || !ids.insert(&item.id)
                || !texts.insert(item.text.to_lowercase())
                || chrono::DateTime::parse_from_rfc3339(&item.created).is_err()
            {
                return Err("Invalid lists file".into());
            }
        }
    }
    Ok(lists)
}
fn write(path: &Path, lists: &Lists) -> Result<(), String> {
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().ok_or("Invalid lists path")?)
        .map_err(|e| e.to_string())?;
    temporary
        .write_all(&serde_json::to_vec(lists).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn request(
    config: &AgentConfig,
    list: ListName,
    change: Option<ListChange>,
) -> Result<Vec<Item>, String> {
    let path = config
        .memory_path
        .as_deref()
        .ok_or("List storage is disabled")?
        .with_file_name("lists.json");
    if change.is_none() {
        return Ok(read(&path)?.items(list).clone());
    }
    let _lock = memory::edit_lock(&path)?;
    let mut lists = read(&path)?;
    let items = lists.items_mut(list);
    match change.unwrap() {
        ListChange::Add { text } => {
            validate(&text)?;
            let text = text.trim();
            if !items
                .iter()
                .any(|item| item.text.to_lowercase() == text.to_lowercase())
            {
                if items.len() >= 200 {
                    return Err("At most 200 items per list".into());
                }
                items.push(Item {
                    id: uuid::Uuid::new_v4().simple().to_string(),
                    text: text.into(),
                    created: chrono::Utc::now().to_rfc3339(),
                });
            }
        }
        ListChange::Remove { expected } => {
            let index = items
                .iter()
                .position(|item| item.id == expected.id)
                .ok_or("Item no longer exists. Refresh the list.")?;
            if items[index] != expected {
                return Err("Item changed. Refresh the list.".into());
            }
            items.remove(index);
        }
        ListChange::Clear {
            expected,
            confirmed,
        } => {
            if !confirmed {
                return Err("Ask the user before clearing this list".into());
            }
            if *items != expected {
                return Err("List changed. Refresh before clearing it.".into());
            }
            items.clear();
        }
    }
    let result = items.clone();
    write(&path, &lists)?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_writers_never_lose_a_successful_item() {
        let dir = tempfile::tempdir().unwrap();
        let config = AgentConfig {
            memory_path: Some(dir.path().join("MEMORY.md")),
            ..Default::default()
        };
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers = (0..2)
            .map(|i| {
                let config = config.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    request(
                        &config,
                        ListName::Todo,
                        Some(ListChange::Add {
                            text: format!("Task {i}"),
                        }),
                    )
                    .is_ok()
                })
            })
            .collect::<Vec<_>>();
        let successes = workers
            .into_iter()
            .map(|worker| usize::from(worker.join().unwrap()))
            .sum::<usize>();
        assert!(successes >= 1);
        assert_eq!(
            request(&config, ListName::Todo, None).unwrap().len(),
            successes
        );
    }
    #[test]
    fn duplicates_limits_conflicts_and_other_lists_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let config = AgentConfig {
            memory_path: Some(dir.path().join("MEMORY.md")),
            ..Default::default()
        };
        let add = |list, text: &str| {
            request(&config, list, Some(ListChange::Add { text: text.into() })).unwrap()
        };
        let first = add(ListName::Shopping, "Milk");
        assert_eq!(add(ListName::Shopping, "milk"), first);
        add(ListName::Shopping, "Bread");
        assert!(
            request(
                &config,
                ListName::Shopping,
                Some(ListChange::Clear {
                    expected: first.clone(),
                    confirmed: true
                })
            )
            .is_err()
        );
        assert!(
            request(
                &config,
                ListName::Shopping,
                Some(ListChange::Clear {
                    expected: request(&config, ListName::Shopping, None).unwrap(),
                    confirmed: false
                })
            )
            .is_err()
        );
        let mut stale = first[0].clone();
        stale.text = "different".into();
        assert!(
            request(
                &config,
                ListName::Shopping,
                Some(ListChange::Remove { expected: stale })
            )
            .is_err()
        );
        let lock = memory::edit_lock(&dir.path().join("lists.json")).unwrap();
        assert!(
            request(
                &config,
                ListName::Todo,
                Some(ListChange::Add {
                    text: "busy".into()
                })
            )
            .is_err()
        );
        drop(lock);
        for i in 0..200 {
            add(ListName::Todo, &format!("Task {i}"));
        }
        assert!(
            request(
                &config,
                ListName::Todo,
                Some(ListChange::Add {
                    text: "Overflow".into()
                })
            )
            .is_err()
        );
        request(
            &config,
            ListName::Shopping,
            Some(ListChange::Remove {
                expected: first[0].clone(),
            }),
        )
        .unwrap();
        assert_eq!(request(&config, ListName::Shopping, None).unwrap().len(), 1);
        assert_eq!(request(&config, ListName::Todo, None).unwrap().len(), 200);
        assert!(
            memory::read(config.memory_path.as_deref().unwrap())
                .unwrap()
                .is_empty()
        );
    }
}
