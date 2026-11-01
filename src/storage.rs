//! Password entries stored directly in an owner-only AIR contract.

use chk::Context;
use ramp::prism::air::{Air, Contract, Id, Instance, Metadata, Name, Pending};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroize;

const MAX_ENTRY_BYTES: usize = 64 * 1024;

#[derive(Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub notes: String,
    pub updated_at: u64,
    pub revision: u64,
}

impl Entry {
    pub fn new() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            username: String::new(),
            password: String::new(),
            url: String::new(),
            notes: String::new(),
            updated_at: 0,
            revision: 0,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), StorageError> {
        if self.title.trim().is_empty() || self.password.is_empty() {
            return Err(StorageError::InvalidEntry);
        }
        let size = self.title.len()
            + self.username.len()
            + self.password.len()
            + self.url.len()
            + self.notes.len();
        if size > MAX_ENTRY_BYTES / 2 || self.title.len() > 256 {
            return Err(StorageError::EntryTooLarge);
        }
        Ok(())
    }
}

impl Default for Entry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("id", &self.id)
            .field("contents", &"[REDACTED]")
            .finish()
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        self.id.zeroize();
        self.title.zeroize();
        self.username.zeroize();
        self.password.zeroize();
        self.url.zeroize();
        self.notes.zeroize();
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
pub struct PasswordContract {
    owner: Name,
    records: BTreeMap<String, Entry>,
    tombstones: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Hash)]
pub enum PasswordAction {
    Put {
        entry: Entry,
        expected_revision: u64,
    },
    Delete {
        id: String,
        expected_revision: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitError {
    Unauthorized,
    InvalidData,
    Conflict,
}
type CommitResult = Result<(), CommitError>;

impl Contract for PasswordContract {
    type Init = ();
    type Message = PasswordAction;
    type Result = CommitResult;

    fn id() -> Id {
        Id::hash("org.ramp.pickle-passwords.passwords.v1")
    }

    fn init(_: (), metadata: Metadata) -> Self {
        Self {
            owner: metadata.signer,
            records: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        }
    }

    fn apply(&mut self, message: PasswordAction, metadata: Metadata) -> CommitResult {
        if metadata.signer != self.owner {
            return Err(CommitError::Unauthorized);
        }
        match message {
            PasswordAction::Put {
                entry,
                expected_revision,
            } => {
                if !valid_id(&entry.id)
                    || entry.validate().is_err()
                    || expected_revision.checked_add(1) != Some(entry.revision)
                {
                    return Err(CommitError::InvalidData);
                }
                if self.tombstones.contains_key(&entry.id)
                    || self
                        .records
                        .get(&entry.id)
                        .map_or(0, |entry| entry.revision)
                        != expected_revision
                {
                    return Err(CommitError::Conflict);
                }
                self.records.insert(entry.id.clone(), entry);
            }
            PasswordAction::Delete {
                id,
                expected_revision,
            } => {
                if self.records.get(&id).map(|entry| entry.revision) != Some(expected_revision) {
                    return Err(CommitError::Conflict);
                }
                let revision = expected_revision
                    .checked_add(1)
                    .ok_or(CommitError::InvalidData)?;
                self.records.remove(&id);
                self.tombstones.insert(id, revision);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    InvalidEntry,
    EntryTooLarge,
    MissingEntry,
    Conflict,
    Randomness,
    InvalidWordlist,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidEntry => "Enter a password.",
            Self::EntryTooLarge => "This entry is too long. Shorten the notes or link.",
            Self::MissingEntry => "This login no longer exists.",
            Self::Conflict => "This login changed on another device. Reopen it before editing.",
            Self::Randomness => "Secure randomness is unavailable. Please try again.",
            Self::InvalidWordlist => "The password wordlist is unavailable.",
        })
    }
}
impl std::error::Error for StorageError {}

pub struct PasswordStore {
    instance: Instance<PasswordContract>,
    receipts: Vec<Pending<CommitResult>>,
}
impl PasswordStore {
    pub fn new(ctx: &Context) -> Self {
        Self::from_air(&ctx.air)
    }

    pub(crate) fn from_air(air: &Air) -> Self {
        let instance = air
            .instances::<PasswordContract>()
            .list()
            .into_values()
            .find(|instance| instance.pending().owner == air.me())
            .unwrap_or_else(|| air.instances::<PasswordContract>().create(()));
        Self {
            instance,
            receipts: Vec::new(),
        }
    }

    pub fn instance(&self) -> Instance<PasswordContract> {
        self.instance.clone()
    }

    fn check_updates(&mut self) -> Result<(), StorageError> {
        let mut rejected = false;
        self.receipts.retain(|receipt| {
            if receipt.pending().is_err() {
                rejected = true;
                return false;
            }
            match receipt.try_confirmed() {
                Some(Err(_)) => {
                    rejected = true;
                    false
                }
                Some(Ok(())) => false,
                None => true,
            }
        });
        if rejected {
            Err(StorageError::Conflict)
        } else {
            Ok(())
        }
    }

    pub fn entries(&mut self) -> Result<Vec<Entry>, StorageError> {
        self.check_updates()?;
        let mut entries: Vec<_> = self.instance.pending().records.values().cloned().collect();
        entries.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(entries)
    }

    pub fn save(&mut self, mut entry: Entry) -> Result<String, StorageError> {
        self.check_updates()?;
        entry.validate()?;
        if entry.id.is_empty() {
            entry.id = random_id()?;
            entry.revision = 0;
        } else if !valid_id(&entry.id) {
            return Err(StorageError::InvalidEntry);
        }
        entry.updated_at = now();
        let expected_revision = entry.revision;
        entry.revision = expected_revision
            .checked_add(1)
            .ok_or(StorageError::Conflict)?;
        let id = entry.id.clone();
        let receipt = self.instance.send(PasswordAction::Put {
            entry,
            expected_revision,
        });
        if receipt.pending().is_err() {
            return Err(StorageError::Conflict);
        }
        self.receipts.push(receipt);
        Ok(id)
    }

    /// A received snapshot has its own deterministic local ID. Retrying an
    /// acceptance must neither duplicate the login nor replace later edits.
    pub(crate) fn import_shared(
        &mut self,
        transfer: Id,
        mut entry: Entry,
    ) -> Result<String, StorageError> {
        self.check_updates()?;
        entry.validate()?;
        let id = shared_entry_id(transfer);
        let snapshot = self.instance.pending();
        if snapshot.records.contains_key(&id) || snapshot.tombstones.contains_key(&id) {
            return Ok(id);
        }
        drop(snapshot);
        entry.id = id;
        entry.revision = 0;
        self.save(entry)
    }

    pub(crate) fn shared_import_confirmed(&self, transfer: Id) -> bool {
        let id = shared_entry_id(transfer);
        self.instance.confirmed().is_some_and(|snapshot| {
            snapshot.records.contains_key(&id) || snapshot.tombstones.contains_key(&id)
        })
    }

    pub fn delete(&mut self, id: &str) -> Result<(), StorageError> {
        self.check_updates()?;
        let revision = self
            .instance
            .pending()
            .records
            .get(id)
            .map(|entry| entry.revision)
            .ok_or(StorageError::MissingEntry)?;
        let receipt = self.instance.send(PasswordAction::Delete {
            id: id.to_owned(),
            expected_revision: revision,
        });
        if receipt.pending().is_err() {
            return Err(StorageError::Conflict);
        }
        self.receipts.push(receipt);
        Ok(())
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn shared_entry_id(transfer: Id) -> String {
    Id::hash(&("peanut-butter-passwords.received-login.v1", transfer)).to_string()[..32].to_owned()
}

fn random_bytes<const N: usize>() -> Result<[u8; N], StorageError> {
    let mut bytes = [0; N];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| StorageError::Randomness)?;
    Ok(bytes)
}

fn random_id() -> Result<String, StorageError> {
    Ok(random_bytes::<16>()?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ramp::prism::{
        air::{ContractType, Request, Value},
        service::{Key, ServiceInit},
        traits,
    };
    use std::{
        any::Any,
        collections::HashMap,
        str::FromStr,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    /// Exercises the public typed AIR facade and real contract without opening sockets.
    #[derive(Clone, Debug)]
    pub(crate) struct MemoryAir(
        Arc<Mutex<Option<PasswordContract>>>,
        Arc<Mutex<HashMap<Id, crate::sharing::LoginTransfer>>>,
        Arc<AtomicBool>,
        Arc<Mutex<HashMap<Id, chk::profiles::Profile>>>,
    );
    impl Default for MemoryAir {
        fn default() -> Self {
            Self(
                Default::default(),
                Default::default(),
                Arc::new(AtomicBool::new(true)),
                Default::default(),
            )
        }
    }

    impl traits::Air for MemoryAir {
        fn me(&self) -> Name {
            owner()
        }
        fn instances(&self, contract: ContractType) -> Box<dyn traits::DynInstances> {
            if contract.id == crate::sharing::LoginTransfer::id() {
                return Box::new(MemoryTransfers(self.1.clone()));
            }
            if contract.id == chk::profiles::Profile::id() {
                return Box::new(MemoryProfiles(self.3.clone()));
            }
            assert_eq!(contract.id, PasswordContract::id());
            Box::new(self.clone())
        }
        fn start(&self, _: ServiceInit) {
            panic!("tests never start network services")
        }
        fn shutdown(&self, _: Key) {}
    }

    impl traits::DynInstances for MemoryAir {
        fn create(&self, init: Request) -> Box<dyn traits::DynInstance> {
            assert!(init.downcast_ref::<()>().is_some());
            self.0
                .lock()
                .unwrap()
                .get_or_insert_with(|| PasswordContract::init((), metadata(owner())));
            Box::new(self.clone())
        }
        fn list(&self) -> HashMap<Id, Box<dyn traits::DynInstance>> {
            if self.0.lock().unwrap().is_none() {
                return HashMap::new();
            }
            HashMap::from([(
                Id::hash("memory-passwords"),
                Box::new(self.clone()) as Box<dyn traits::DynInstance>,
            )])
        }
        fn try_next(&mut self) -> Option<Box<dyn traits::DynInstance>> {
            None
        }
        fn next(&mut self) -> traits::BoxFuture<'_, Box<dyn traits::DynInstance>> {
            Box::pin(std::future::pending())
        }
    }

    #[derive(Debug)]
    struct Snapshot<T: fmt::Debug + Send + Sync + 'static>(T);
    impl<T: fmt::Debug + Send + Sync + 'static> traits::DynRef for Snapshot<T> {
        fn value(&self) -> &(dyn Any + Send + Sync) {
            &self.0
        }
    }

    impl traits::DynInstance for MemoryAir {
        fn id(&self) -> Id {
            Id::hash("memory-passwords")
        }
        fn notify(&self, _: bool) {
            panic!("password store must not enable notifications")
        }
        fn share(&self, _: Name) {
            panic!("password store must not share credentials")
        }
        fn send(&mut self, message: Request) -> Box<dyn traits::DynPending> {
            let action = *message.downcast::<PasswordAction>().unwrap();
            let result = self
                .0
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .apply(action, metadata(owner()));
            Box::new(Receipt(result))
        }
        fn pending(&self) -> Box<dyn traits::DynRef + '_> {
            Box::new(Snapshot(self.0.lock().unwrap().clone().unwrap()))
        }
        fn confirmed(&self) -> Option<Box<dyn traits::DynRef + '_>> {
            self.2.load(Ordering::Relaxed).then(|| self.pending())
        }
        fn try_next(&mut self) -> Option<traits::ContractUpdate> {
            None
        }
        fn next(&mut self) -> traits::BoxFuture<'_, traits::ContractUpdate> {
            Box::pin(std::future::pending())
        }
    }

    #[derive(Clone, Debug)]
    struct Receipt<T: Clone + fmt::Debug + Send + Sync + 'static>(T);
    impl<T: Clone + fmt::Debug + Send + Sync + 'static> traits::DynPending for Receipt<T> {
        fn pending(&self) -> Box<dyn traits::DynRef + '_> {
            Box::new(Snapshot(self.0.clone()))
        }
        fn try_confirmed(&self) -> Option<Value> {
            Some(Arc::new(self.0.clone()))
        }
        fn try_next(&mut self) -> Option<bool> {
            None
        }
        fn next(&mut self) -> traits::BoxFuture<'_, bool> {
            Box::pin(std::future::pending())
        }
    }

    #[derive(Clone, Debug)]
    struct MemoryTransfers(Arc<Mutex<HashMap<Id, crate::sharing::LoginTransfer>>>);
    impl traits::DynInstances for MemoryTransfers {
        fn create(&self, init: Request) -> Box<dyn traits::DynInstance> {
            let init = *init
                .downcast::<crate::sharing::LoginTransferInit>()
                .unwrap();
            let id = Id::hash(&init.nonce);
            self.0
                .lock()
                .unwrap()
                .entry(id)
                .or_insert_with(|| crate::sharing::LoginTransfer::init(init, metadata(owner())));
            Box::new(MemoryTransfer(self.0.clone(), id))
        }
        fn list(&self) -> HashMap<Id, Box<dyn traits::DynInstance>> {
            self.0
                .lock()
                .unwrap()
                .keys()
                .map(|id| {
                    (
                        *id,
                        Box::new(MemoryTransfer(self.0.clone(), *id))
                            as Box<dyn traits::DynInstance>,
                    )
                })
                .collect()
        }
        fn try_next(&mut self) -> Option<Box<dyn traits::DynInstance>> {
            None
        }
        fn next(&mut self) -> traits::BoxFuture<'_, Box<dyn traits::DynInstance>> {
            Box::pin(std::future::pending())
        }
    }
    #[derive(Clone, Debug)]
    struct MemoryTransfer(Arc<Mutex<HashMap<Id, crate::sharing::LoginTransfer>>>, Id);
    impl traits::DynInstance for MemoryTransfer {
        fn id(&self) -> Id {
            self.1
        }
        fn notify(&self, _: bool) {
            panic!("tests never enable notifications");
        }
        fn share(&self, recipient: Name) {
            assert_eq!(
                self.0.lock().unwrap().get(&self.1).unwrap().recipient,
                recipient
            );
        }
        fn send(&mut self, message: Request) -> Box<dyn traits::DynPending> {
            let action = *message
                .downcast::<crate::sharing::TransferAction>()
                .unwrap();
            let result = self
                .0
                .lock()
                .unwrap()
                .get_mut(&self.1)
                .unwrap()
                .apply(action, metadata(owner()));
            Box::new(Receipt(result))
        }
        fn pending(&self) -> Box<dyn traits::DynRef + '_> {
            Box::new(Snapshot(
                self.0.lock().unwrap().get(&self.1).unwrap().clone(),
            ))
        }
        fn confirmed(&self) -> Option<Box<dyn traits::DynRef + '_>> {
            Some(self.pending())
        }
        fn try_next(&mut self) -> Option<traits::ContractUpdate> {
            None
        }
        fn next(&mut self) -> traits::BoxFuture<'_, traits::ContractUpdate> {
            Box::pin(std::future::pending())
        }
    }

    #[derive(Clone, Debug)]
    struct MemoryProfiles(Arc<Mutex<HashMap<Id, chk::profiles::Profile>>>);
    impl traits::DynInstances for MemoryProfiles {
        fn create(&self, init: Request) -> Box<dyn traits::DynInstance> {
            let name = *init.downcast::<Name>().unwrap();
            let id = Id::random();
            self.0
                .lock()
                .unwrap()
                .insert(id, chk::profiles::Profile::init(name, metadata(owner())));
            Box::new(MemoryProfile(self.0.clone(), id))
        }
        fn list(&self) -> HashMap<Id, Box<dyn traits::DynInstance>> {
            self.0
                .lock()
                .unwrap()
                .keys()
                .map(|id| {
                    (
                        *id,
                        Box::new(MemoryProfile(self.0.clone(), *id))
                            as Box<dyn traits::DynInstance>,
                    )
                })
                .collect()
        }
        fn try_next(&mut self) -> Option<Box<dyn traits::DynInstance>> {
            None
        }
        fn next(&mut self) -> traits::BoxFuture<'_, Box<dyn traits::DynInstance>> {
            Box::pin(std::future::pending())
        }
    }
    #[derive(Clone, Debug)]
    struct MemoryProfile(Arc<Mutex<HashMap<Id, chk::profiles::Profile>>>, Id);
    impl traits::DynInstance for MemoryProfile {
        fn id(&self) -> Id {
            self.1
        }
        fn notify(&self, _: bool) {
            panic!("saving a contact must not enable notifications");
        }
        fn share(&self, _: Name) {
            panic!("saving a contact must not share its profile");
        }
        fn send(&mut self, message: Request) -> Box<dyn traits::DynPending> {
            let action = *message.downcast::<chk::profiles::ProfileAction>().unwrap();
            self.0
                .lock()
                .unwrap()
                .get_mut(&self.1)
                .unwrap()
                .apply(action, metadata(owner()));
            Box::new(Receipt(()))
        }
        fn pending(&self) -> Box<dyn traits::DynRef + '_> {
            Box::new(Snapshot(
                self.0.lock().unwrap().get(&self.1).unwrap().clone(),
            ))
        }
        fn confirmed(&self) -> Option<Box<dyn traits::DynRef + '_>> {
            Some(self.pending())
        }
        fn try_next(&mut self) -> Option<traits::ContractUpdate> {
            None
        }
        fn next(&mut self) -> traits::BoxFuture<'_, traits::ContractUpdate> {
            Box::pin(std::future::pending())
        }
    }

    impl MemoryAir {
        pub(crate) fn profile(&self, profile: chk::profiles::Profile) -> Id {
            let id = Id::random();
            self.3.lock().unwrap().insert(id, profile);
            id
        }
        pub(crate) fn confirm_private(&self, confirmed: bool) {
            self.2.store(confirmed, Ordering::Relaxed);
        }
        /// Simulate an already-arrived immutable copy in recipient UI tests.
        pub(crate) fn received(&self, mut transfer: crate::sharing::LoginTransfer) -> Id {
            transfer.recipient = owner();
            let id = Id::random();
            self.1.lock().unwrap().insert(id, transfer);
            id
        }
    }

    fn owner() -> Name {
        Name::orange_me()
    }
    fn metadata(signer: Name) -> Metadata {
        Metadata {
            signer,
            timestamp: 1,
            confirmed: true,
        }
    }
    fn entry() -> Entry {
        let mut entry = Entry::new();
        entry.title = "Example".into();
        entry.username = "person@example.test".into();
        entry.password = "rainy-october-simply".into();
        entry.url = "https://example.test".into();
        entry.notes = "Optional notes".into();
        entry
    }

    #[test]
    fn air_entries_restore_directly_and_preserve_edits_and_deletions() {
        let memory = MemoryAir::default();
        let air = Air::new(memory.clone());
        let mut session = PasswordStore::from_air(&air);
        assert!(session.entries().unwrap().is_empty());
        let id = session.save(entry()).unwrap();
        let mut edited = session.entries().unwrap().remove(0);
        let stale = edited.clone();
        edited.password = "updated password".into();
        session.save(edited).unwrap();
        assert_eq!(session.save(stale.clone()), Err(StorageError::Conflict));

        let cache = serde_json::to_vec(&*memory.0.lock().unwrap()).unwrap();
        let restored_memory = MemoryAir(
            Arc::new(Mutex::new(serde_json::from_slice(&cache).unwrap())),
            Default::default(),
            Arc::new(AtomicBool::new(true)),
            Default::default(),
        );
        let mut restored = PasswordStore::from_air(&Air::new(restored_memory.clone()));
        let entries = restored.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].password, "updated password");
        assert_eq!(entries[0].username, "person@example.test");
        assert_eq!(entries[0].url, "https://example.test");
        assert_eq!(entries[0].notes, "Optional notes");
        assert_eq!(entries[0].revision, 2);
        restored.delete(&id).unwrap();
        assert!(restored.entries().unwrap().is_empty());
        assert_eq!(restored.save(stale), Err(StorageError::Conflict));
        let mut reopened = PasswordStore::from_air(&Air::new(restored_memory));
        assert!(reopened.entries().unwrap().is_empty());
    }

    #[test]
    fn shared_import_is_idempotent_preserves_edits_and_never_revives_deleted_copies() {
        let mut store = PasswordStore::from_air(&Air::new(MemoryAir::default()));
        let transfer = Id::random();
        let id = store.import_shared(transfer, entry()).unwrap();
        assert!(valid_id(&id));
        assert_eq!(store.import_shared(transfer, entry()).unwrap(), id);
        let mut local = store.entries().unwrap().remove(0);
        local.username = "edited locally".into();
        store.save(local).unwrap();
        store.import_shared(transfer, entry()).unwrap();
        let entries = store.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].username, "edited locally");
        store.delete(&id).unwrap();
        store.import_shared(transfer, entry()).unwrap();
        assert!(store.entries().unwrap().is_empty());
        let another = store.import_shared(Id::random(), entry()).unwrap();
        assert_ne!(another, id);
    }

    #[test]
    fn contract_rejects_foreign_writes_and_invalid_revisions() {
        let mut contract = PasswordContract::init((), metadata(owner()));
        let outsider =
            Name::from_str("0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
                .unwrap();
        let mut entry = entry();
        entry.id = "0123456789abcdef0123456789abcdef".into();
        entry.revision = 1;
        let put = PasswordAction::Put {
            entry: entry.clone(),
            expected_revision: 0,
        };
        assert_eq!(
            contract.apply(put.clone(), metadata(outsider)),
            Err(CommitError::Unauthorized)
        );
        assert!(contract.records.is_empty());
        contract.apply(put.clone(), metadata(owner())).unwrap();
        assert_eq!(
            contract.apply(put, metadata(owner())),
            Err(CommitError::Conflict)
        );
        assert_eq!(
            contract.apply(
                PasswordAction::Delete {
                    id: entry.id.clone(),
                    expected_revision: 1
                },
                metadata(outsider)
            ),
            Err(CommitError::Unauthorized)
        );
        entry.revision = 99;
        assert_eq!(
            contract.apply(
                PasswordAction::Put {
                    entry,
                    expected_revision: 1
                },
                metadata(owner())
            ),
            Err(CommitError::InvalidData)
        );
        assert_eq!(contract.records.len(), 1);
    }
}
