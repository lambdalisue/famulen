//! The vault: secrets sealed with an age key only this process holds, one
//! file per secret in a directory of their own. Anyone may put a secret in or
//! take one out; only the gateway reads one, when a call to the outside
//! needs it. The owner can open a secret without the gateway, with their key
//! and an age tool.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use age::secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::store::now;

/// The environment variable the key is read from.
pub const KEY_VAR: &str = "FAMULEN_VAULT_KEY";

/// Who wrote a secret the gateway put in itself, from a login or a
/// connection's key.
pub const GATEWAY: &str = "gateway";

/// The list of secrets beside the sealed files. It holds no secret.
const INDEX: &str = "index.toml";

/// The key secrets are sealed with: an age X25519 identity
/// (`AGE-SECRET-KEY-1…`).
#[derive(Clone)]
pub struct VaultKey(Arc<age::x25519::Identity>);

impl VaultKey {
    pub fn parse(text: impl AsRef<str>) -> anyhow::Result<Self> {
        let text = text.as_ref().trim();
        text.parse::<age::x25519::Identity>()
            .map(|identity| Self(Arc::new(identity)))
            .map_err(|_| {
                anyhow::anyhow!(
                    "{KEY_VAR} is not an age key (AGE-SECRET-KEY-1…); set it to a new one such as {}",
                    Self::generate()
                )
            })
    }

    /// A new key, written the way [`Self::parse`] reads it.
    pub fn generate() -> String {
        age::x25519::Identity::generate()
            .to_string()
            .expose_secret()
            .to_owned()
    }

    fn recipient(&self) -> age::x25519::Recipient {
        self.0.to_public()
    }
}

impl PartialEq for VaultKey {
    fn eq(&self, other: &Self) -> bool {
        self.recipient().to_string() == other.recipient().to_string()
    }
}

impl Eq for VaultKey {}

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultKey(…)")
    }
}

/// A secret as anyone but the gateway sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultEntry {
    pub name: String,
    /// The connection or model that uses it, if any.
    pub used_by: String,
    /// What it is for, in the words of whoever described it.
    pub description: String,
    /// Who wrote it last: `owner`, `secretary`, or `gateway` for what a
    /// login or a connection's key put in.
    pub by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    secrets: BTreeMap<String, Kept>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Kept {
    file: String,
    used_by: String,
    #[serde(default)]
    description: String,
    by: String,
    created_at: i64,
    updated_at: i64,
}

impl Kept {
    fn entry(self, name: String) -> VaultEntry {
        VaultEntry {
            name,
            used_by: self.used_by,
            description: self.description,
            by: self.by,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Clone)]
pub struct Vault {
    dir: PathBuf,
    key: VaultKey,
    /// Writes to the index one at a time.
    lock: Arc<std::sync::Mutex<()>>,
}

impl Vault {
    /// Open the vault in `dir`, making it readable only by its owner when it
    /// is new.
    pub fn open(dir: impl Into<PathBuf>, key: &VaultKey) -> anyhow::Result<Self> {
        let dir = dir.into();
        if !dir.exists() {
            std::fs::create_dir_all(&dir)?;
            restrict(&dir, 0o700)?;
        }
        Ok(Self {
            dir,
            key: key.clone(),
            lock: Arc::default(),
        })
    }

    /// Where the sealed files are.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Put a secret in, replacing one of the same name and keeping its
    /// description.
    pub fn put(
        &self,
        name: impl AsRef<str>,
        value: impl AsRef<str>,
        used_by: impl AsRef<str>,
        by: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        let name = name.as_ref().trim();
        anyhow::ensure!(!name.is_empty(), "a secret needs a name");
        anyhow::ensure!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')),
            "a secret's name is letters, digits and - _ : .: {name}"
        );
        let value = value.as_ref();
        anyhow::ensure!(!value.is_empty(), "a secret cannot be empty");
        let sealed = age::encrypt(&self.key.recipient(), value.as_bytes())
            .map_err(|_| anyhow::anyhow!("could not seal {name}"))?;
        let _guard = crate::lock::lock(&self.lock);
        let mut index = self.index()?;
        let file = file_name(name);
        if let Some((other, _)) = index
            .secrets
            .iter()
            .find(|(n, k)| k.file == file && n.as_str() != name)
        {
            anyhow::bail!("{name} would share a file with {other}");
        }
        replace(&self.dir.join(&file), &sealed, 0o600)?;
        let at = now();
        let (created_at, description) = index.secrets.get(name).map_or((at, String::new()), |k| {
            (k.created_at, k.description.clone())
        });
        index.secrets.insert(
            name.to_owned(),
            Kept {
                file,
                used_by: used_by.as_ref().to_owned(),
                description,
                by: by.as_ref().to_owned(),
                created_at,
                updated_at: at,
            },
        );
        self.write_index(&index)
    }

    /// Say what a secret is for, leaving its value as it is.
    pub fn describe(
        &self,
        name: impl AsRef<str>,
        description: impl AsRef<str>,
        by: impl AsRef<str>,
    ) -> anyhow::Result<()> {
        let name = name.as_ref();
        let _guard = crate::lock::lock(&self.lock);
        let mut index = self.index()?;
        let kept = index
            .secrets
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("no secret {name}"))?;
        kept.description = description.as_ref().trim().to_owned();
        kept.by = by.as_ref().to_owned();
        kept.updated_at = now();
        self.write_index(&index)
    }

    /// Take a secret out. Whether there was one.
    pub fn delete(&self, name: impl AsRef<str>) -> anyhow::Result<bool> {
        let _guard = crate::lock::lock(&self.lock);
        let mut index = self.index()?;
        let Some(kept) = index.secrets.remove(name.as_ref()) else {
            return Ok(false);
        };
        self.write_index(&index)?;
        match std::fs::remove_file(self.dir.join(&kept.file)) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error.into()),
        }
    }

    pub fn list(&self) -> anyhow::Result<Vec<VaultEntry>> {
        Ok(self
            .index()?
            .secrets
            .into_iter()
            .map(|(name, k)| k.entry(name))
            .collect())
    }

    /// One secret as [`Self::list`] shows it.
    pub fn get(&self, name: impl AsRef<str>) -> anyhow::Result<Option<VaultEntry>> {
        let name = name.as_ref();
        Ok(self
            .index()?
            .secrets
            .remove(name)
            .map(|k| k.entry(name.to_owned())))
    }

    /// The secret itself, for a call to the outside.
    pub(super) fn reveal(&self, name: impl AsRef<str>) -> anyhow::Result<Option<String>> {
        let name = name.as_ref();
        let Some(kept) = self.index()?.secrets.get(name).cloned() else {
            return Ok(None);
        };
        let sealed = std::fs::read(self.dir.join(&kept.file))?;
        let open = age::decrypt(self.key.0.as_ref(), &sealed)
            .map_err(|_| anyhow::anyhow!("{name} cannot be opened with this {KEY_VAR}"))?;
        Ok(Some(String::from_utf8(open)?))
    }

    fn index(&self) -> anyhow::Result<Index> {
        match std::fs::read_to_string(self.dir.join(INDEX)) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Index::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn write_index(&self, index: &Index) -> anyhow::Result<()> {
        replace(
            &self.dir.join(INDEX),
            toml::to_string(index)?.as_bytes(),
            0o600,
        )
    }
}

/// The file a secret is sealed in: its name with `:` as `-`, and `.age`.
fn file_name(name: &str) -> String {
    format!("{}.age", name.replace(':', "-"))
}

/// Write `bytes` to `path` whole: to a file beside it first, then in its
/// place, so a write cut off leaves the old one.
fn replace(path: &Path, bytes: &[u8], mode: u32) -> anyhow::Result<()> {
    let temp = path.with_extension(format!("tmp-{}", rand::random::<u64>()));
    {
        let mut file = std::fs::File::create(&temp)?;
        restrict(&temp, mode)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh directory for one test, removed when the value is dropped.
    pub(crate) struct TempDir(pub(crate) PathBuf);

    impl TempDir {
        pub(crate) fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "famulen-vault-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn key() -> VaultKey {
        VaultKey::parse(VaultKey::generate()).unwrap()
    }

    /// A vault in a directory of its own that lives as long as the tests do.
    pub(crate) fn scratch() -> Vault {
        let dir = TempDir::new();
        let vault = Vault::open(dir.0.join("vault"), &key()).unwrap();
        std::mem::forget(dir);
        vault
    }

    #[test]
    fn a_secret_put_in_is_listed_without_any_part_of_its_value_and_only_revealed_inside() {
        let dir = TempDir::new();
        let v = Vault::open(dir.0.join("vault"), &key()).unwrap();

        v.put(
            "connection:clickup-team:api_key",
            "pk_1234567890ABCDEFGHIJKLMNOP",
            "clickup-team",
            GATEWAY,
        )
        .unwrap();

        let entries = v.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "connection:clickup-team:api_key");
        assert_eq!(entries[0].used_by, "clickup-team");
        let listed = serde_json::to_string(&entries).unwrap();
        let index = std::fs::read_to_string(dir.0.join("vault").join(INDEX)).unwrap();
        for fragment in ["pk_1", "MNOP", "…"] {
            assert!(!listed.contains(fragment), "{fragment} in {listed}");
            assert!(!index.contains(fragment), "{fragment} in {index}");
        }
        assert_eq!(
            v.reveal("connection:clickup-team:api_key")
                .unwrap()
                .as_deref(),
            Some("pk_1234567890ABCDEFGHIJKLMNOP")
        );
    }

    #[test]
    fn each_secret_is_its_own_age_file_that_the_owners_key_opens() {
        let dir = TempDir::new();
        let k = key();
        let v = Vault::open(dir.0.join("vault"), &k).unwrap();
        v.put("pw", "correct horse battery staple", "", "owner")
            .unwrap();

        let sealed = std::fs::read(dir.0.join("vault").join("pw.age")).unwrap();
        assert!(sealed.starts_with(b"age-encryption.org/v1"));
        assert!(!sealed.windows(b"horse".len()).any(|w| w == b"horse"));
        let index = std::fs::read_to_string(dir.0.join("vault").join(INDEX)).unwrap();
        assert!(!index.contains("horse"));
        let opened = age::decrypt(k.0.as_ref(), &sealed).unwrap();
        assert_eq!(opened, b"correct horse battery staple");
    }

    #[cfg(unix)]
    #[test]
    fn a_new_vault_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let v = Vault::open(dir.0.join("vault"), &key()).unwrap();
        v.put("pw", "secret", "", "owner").unwrap();
        let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.0.join("vault")), 0o700);
        assert_eq!(mode(dir.0.join("vault").join("pw.age")), 0o600);
    }

    #[test]
    fn another_key_cannot_open_a_secret() {
        let dir = TempDir::new();
        Vault::open(dir.0.join("vault"), &key())
            .unwrap()
            .put("pw", "secret", "", "owner")
            .unwrap();

        let error = Vault::open(dir.0.join("vault"), &key())
            .unwrap()
            .reveal("pw")
            .unwrap_err();

        assert!(error.to_string().contains(KEY_VAR), "{error}");
    }

    #[test]
    fn putting_again_replaces_and_deleting_removes_the_file() {
        let dir = TempDir::new();
        let v = Vault::open(dir.0.join("vault"), &key()).unwrap();
        v.put("pw", "first", "", "owner").unwrap();
        v.put("pw", "second", "", "owner").unwrap();
        assert_eq!(v.reveal("pw").unwrap().as_deref(), Some("second"));
        assert_eq!(v.list().unwrap().len(), 1);
        let leftovers = std::fs::read_dir(dir.0.join("vault")).unwrap().count();
        assert_eq!(
            leftovers, 2,
            "the sealed file and the index, no temporary files"
        );

        assert!(v.delete("pw").unwrap());
        assert!(!v.delete("pw").unwrap());
        assert_eq!(v.reveal("pw").unwrap(), None);
        assert!(!dir.0.join("vault").join("pw.age").exists());
    }

    #[test]
    fn a_key_is_an_age_identity_and_another_form_is_refused_with_a_suggestion() {
        assert!(VaultKey::generate().starts_with("AGE-SECRET-KEY-1"));
        assert!(VaultKey::parse(VaultKey::generate()).is_ok());
        let error = VaultKey::parse("c2hvcnQ=").unwrap_err().to_string();
        assert!(error.contains("AGE-SECRET-KEY-1"), "{error}");
        assert_eq!(format!("{:?}", key()), "VaultKey(…)");
    }

    #[test]
    fn the_list_says_who_wrote_a_secret_and_keeps_when_it_was_first_put() {
        let dir = TempDir::new();
        let v = Vault::open(dir.0.join("vault"), &key()).unwrap();
        v.put("pw", "first", "bank", "owner").unwrap();
        let first = v.list().unwrap().remove(0);

        v.put("pw", "second", "bank", "secretary").unwrap();

        let entry = v.list().unwrap().remove(0);
        assert_eq!(entry.by, "secretary");
        assert_eq!(entry.used_by, "bank");
        assert_eq!(entry.created_at, first.created_at);
        assert!(entry.updated_at >= first.updated_at);
        assert_eq!(
            v.get("pw").unwrap().map(|e| e.by).as_deref(),
            Some("secretary")
        );
        assert_eq!(v.get("nothing").unwrap(), None);
    }

    #[test]
    fn describing_a_secret_keeps_its_value_and_replacing_the_value_keeps_its_description() {
        let dir = TempDir::new();
        let v = Vault::open(dir.0.join("vault"), &key()).unwrap();
        v.put("pw", "first", "", "owner").unwrap();
        let sealed = std::fs::read(dir.0.join("vault").join("pw.age")).unwrap();

        v.describe("pw", "  銀行のサイト  ", "secretary").unwrap();

        let described = v.get("pw").unwrap().unwrap();
        assert_eq!(described.description, "銀行のサイト");
        assert_eq!(described.by, "secretary");
        assert_eq!(v.reveal("pw").unwrap().as_deref(), Some("first"));
        assert_eq!(
            std::fs::read(dir.0.join("vault").join("pw.age")).unwrap(),
            sealed
        );

        v.put("pw", "second", "", "owner").unwrap();
        assert_eq!(v.get("pw").unwrap().unwrap().description, "銀行のサイト");
    }

    #[test]
    fn describing_a_secret_that_is_not_there_is_refused() {
        let v = scratch();
        let error = v.describe("nothing", "x", "owner").unwrap_err();
        assert!(error.to_string().contains("no secret nothing"), "{error}");
        assert!(v.list().unwrap().is_empty());
    }

    #[test]
    fn an_index_written_with_masked_forms_is_still_read_and_rewritten_without_them() {
        let dir = TempDir::new();
        let v = Vault::open(dir.0.join("vault"), &key()).unwrap();
        v.put("pw", "first", "bank", "owner").unwrap();
        let index = dir.0.join("vault").join(INDEX);
        let old = std::fs::read_to_string(&index).unwrap().replace(
            "used_by = \"bank\"",
            "used_by = \"bank\"\nmasked = \"firs…\"",
        );
        std::fs::write(&index, old).unwrap();

        assert_eq!(v.list().unwrap()[0].name, "pw");
        v.describe("pw", "the bank", "owner").unwrap();
        assert!(!std::fs::read_to_string(&index).unwrap().contains("masked"));
    }

    #[test]
    fn an_empty_secret_or_a_bad_name_is_refused() {
        let dir = TempDir::new();
        let v = Vault::open(dir.0.join("vault"), &key()).unwrap();
        assert!(v.put("", "x", "", "owner").is_err());
        assert!(v.put("pw", "", "", "owner").is_err());
        assert!(v.put("../escape", "x", "", "owner").is_err());
    }
}
