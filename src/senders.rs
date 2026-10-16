//! Sender registry, loaded from a JSON config file at startup.
//! No addresses are hardcoded — see senders.example.json.

use std::path::Path;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::validation;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Sender {
    /// Short handle used as `from` in send_email (e.g. "me", "support").
    pub id: String,
    pub email: String,
    /// Shown in the From header: "Display Name <email>".
    pub display_name: String,
}

impl Sender {
    pub fn as_from_header(&self) -> String {
        format!("{} <{}>", self.display_name, self.email)
    }
}

/// Load and validate senders from a JSON file of the shape:
/// `[{"id": "me", "email": "you@yourdomain.com", "display_name": "You"}]`
pub fn load(path: &Path) -> anyhow::Result<Vec<Sender>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read senders config at {}", path.display()))?;
    let senders: Vec<Sender> = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    validate(&senders).with_context(|| format!("invalid senders config in {}", path.display()))?;
    Ok(senders)
}

fn validate(senders: &[Sender]) -> anyhow::Result<()> {
    if senders.is_empty() {
        bail!("config is empty — define at least one sender");
    }
    let mut ids = std::collections::HashSet::new();
    let mut emails = std::collections::HashSet::new();
    for s in senders {
        if !is_valid_sender_id(&s.id) {
            bail!(
                "invalid sender id {:?}: 2-40 chars of a-z, 0-9, '-', no leading/trailing '-'",
                s.id
            );
        }
        if !validation::is_valid_email(&s.email) {
            bail!("invalid sender email {:?}", s.email);
        }
        let name = s.display_name.trim();
        if name.is_empty() || name.len() > 100 || name.contains(['\r', '\n', '<', '>', '"', '\\']) {
            bail!(
                "invalid display name for sender {:?}: single line, no <, >, quotes or backslashes",
                s.id
            );
        }
        if !ids.insert(s.id.as_str()) {
            bail!("duplicate sender id {:?}", s.id);
        }
        if !emails.insert(s.email.to_ascii_lowercase()) {
            bail!("duplicate sender email {:?}", s.email);
        }
    }
    Ok(())
}

pub fn is_valid_sender_id(id: &str) -> bool {
    (2..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

/// Resolve a `from` argument (sender id or full email, case-insensitive).
pub fn resolve<'a>(senders: &'a [Sender], query: &str) -> Option<&'a Sender> {
    let q = query.trim();
    senders
        .iter()
        .find(|s| s.id.eq_ignore_ascii_case(q) || s.email.eq_ignore_ascii_case(q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender(id: &str, email: &str, name: &str) -> Sender {
        Sender {
            id: id.into(),
            email: email.into(),
            display_name: name.into(),
        }
    }

    #[test]
    fn resolve_by_id_and_email_case_insensitive() {
        let senders = vec![sender("me", "alice@example.com", "Alice")];
        assert_eq!(resolve(&senders, "me").unwrap().email, "alice@example.com");
        assert_eq!(resolve(&senders, "ME").unwrap().id, "me");
        assert_eq!(resolve(&senders, "Alice@Example.COM").unwrap().id, "me");
        assert!(resolve(&senders, "nobody").is_none());
    }

    #[test]
    fn sender_id_rules() {
        for ok in ["me", "support-team", "bot1", "a1"] {
            assert!(is_valid_sender_id(ok), "should accept {ok}");
        }
        for bad in ["", "a", "-me", "me-", "Me", "has space", "under_score"] {
            assert!(!is_valid_sender_id(bad), "should reject {bad:?}");
        }
    }

    #[test]
    fn validation_catches_bad_configs() {
        assert!(validate(&[]).is_err());
        assert!(validate(&[sender("Me", "alice@example.com", "Alice")]).is_err());
        assert!(validate(&[sender("me", "not-an-email", "Alice")]).is_err());
        assert!(validate(&[sender("me", "alice@example.com", "Al\nice")]).is_err());
        assert!(
            validate(&[
                sender("me", "alice@example.com", "Alice"),
                sender("me", "bob@example.com", "Bob"),
            ])
            .is_err()
        );
        assert!(
            validate(&[
                sender("me", "alice@example.com", "Alice"),
                sender("other", "alice@example.com", "Alice"),
            ])
            .is_err()
        );
        assert!(
            validate(&[
                sender("me", "alice@example.com", "Alice"),
                sender("other", "bob@example.com", "Bob"),
            ])
            .is_ok()
        );
    }

    #[test]
    fn load_from_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("senders-test-{}.json", std::process::id()));
        std::fs::write(
            &path,
            r#"[{"id":"me","email":"alice@example.com","display_name":"Alice"}]"#,
        )
        .unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].as_from_header(), "Alice <alice@example.com>");
        std::fs::remove_file(&path).ok();

        std::fs::write(&path, "not json").unwrap();
        assert!(load(&path).is_err());
        std::fs::remove_file(&path).ok();
    }
}
