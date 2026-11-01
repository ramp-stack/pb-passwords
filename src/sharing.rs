//! Explicit immutable login copies, each in a separate AIR contract.
//! The owner's complete PasswordContract is never shared.
use crate::storage::{Entry, PasswordStore};
use chk::Context;
use prism::air::{Air, Contract, Id, Instance, Instances, Metadata, Name};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransferStatus {
    Pending,
    Accepted,
    Declined,
}

#[derive(Clone, Hash, Serialize, Deserialize)]
pub struct LoginTransferInit {
    pub recipient: Name,
    pub nonce: Id,
    pub entry: Entry,
}
impl fmt::Debug for LoginTransferInit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginTransferInit")
            .field("recipient", &self.recipient)
            .field("entry", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Hash, Serialize, Deserialize)]
pub struct LoginTransfer {
    pub sender: Name,
    pub recipient: Name,
    pub entry: Entry,
    pub status: TransferStatus,
    pub created_at: u64,
}
impl fmt::Debug for LoginTransfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginTransfer")
            .field("sender", &self.sender)
            .field("recipient", &self.recipient)
            .field("status", &self.status)
            .field("entry", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Hash, Serialize, Deserialize)]
pub enum TransferAction {
    Accept,
    Decline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferError {
    Unauthorized,
    Invalid,
    Closed,
}
impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unauthorized => "Only the intended recipient can save or decline this login.",
            Self::Invalid => "This shared login is invalid.",
            Self::Closed => "This shared login has already been handled.",
        })
    }
}

impl Contract for LoginTransfer {
    type Init = LoginTransferInit;
    type Message = TransferAction;
    type Result = Result<(), TransferError>;
    fn id() -> Id {
        Id::hash("org.ramp.peanut-butter-passwords.LoginTransfer.v1")
    }
    fn init(init: Self::Init, metadata: Metadata) -> Self {
        Self {
            sender: metadata.signer,
            recipient: init.recipient,
            entry: init.entry,
            status: TransferStatus::Pending,
            created_at: metadata.timestamp / 1_000_000_000,
        }
    }
    fn apply(&mut self, action: Self::Message, metadata: Metadata) -> Self::Result {
        self.validate()?;
        if metadata.signer != self.recipient {
            return Err(TransferError::Unauthorized);
        }
        let status = match action {
            TransferAction::Accept => TransferStatus::Accepted,
            TransferAction::Decline => TransferStatus::Declined,
        };
        if self.status == status {
            return Ok(());
        }
        if self.status != TransferStatus::Pending {
            return Err(TransferError::Closed);
        }
        self.status = status;
        Ok(())
    }
}

impl LoginTransfer {
    pub fn validate(&self) -> Result<(), TransferError> {
        if self.sender == self.recipient || self.entry.validate().is_err() {
            return Err(TransferError::Invalid);
        }
        Ok(())
    }
}

pub fn recipient(text: &str, me: Name) -> Result<Name, String> {
    let name = text
        .trim()
        .parse::<Name>()
        .map_err(|_| "Enter a complete Peanut Butter Name.".to_owned())?;
    if name == me {
        return Err("Choose another person’s Peanut Butter Name.".into());
    }
    Ok(name)
}

pub fn open(air: &Air) -> Instances<LoginTransfer> {
    air.instances::<LoginTransfer>()
}

/// Queues a dedicated copy. AIR creation confirmation is not delivery or
/// recipient acceptance; callers can inspect `confirmed()` and `status`.
pub fn send(
    ctx: &mut Context,
    mut entry: Entry,
    recipient: Name,
) -> Result<Instance<LoginTransfer>, String> {
    if recipient == ctx.air.me() {
        return Err("Choose another person’s Peanut Butter Name.".into());
    }
    entry.validate().map_err(|e| e.to_string())?;
    // Sender-local bookkeeping is not part of a shared credential snapshot.
    entry.id.clear();
    entry.revision = 0;
    entry.updated_at = 0;
    let transfer = open(&ctx.air).create(LoginTransferInit {
        recipient,
        nonce: Id::random(),
        entry,
    });
    transfer.share(recipient);
    Ok(transfer)
}

pub fn receive(
    ctx: &mut Context,
    store: &mut PasswordStore,
    transfer: &mut Instance<LoginTransfer>,
) -> Result<bool, String> {
    receive_into(&ctx.air, store, transfer)
}

fn receive_into(
    air: &Air,
    store: &mut PasswordStore,
    transfer: &mut Instance<LoginTransfer>,
) -> Result<bool, String> {
    let snapshot = transfer.pending();
    snapshot.validate().map_err(|e| e.to_string())?;
    if snapshot.recipient != air.me() {
        return Err(TransferError::Unauthorized.to_string());
    }
    if snapshot.status == TransferStatus::Declined {
        return Err(TransferError::Closed.to_string());
    }
    let entry = snapshot.entry.clone();
    drop(snapshot);
    // Re-run the idempotent import even after acceptance, so an interrupted
    // acceptance can repair a missing local copy without clobbering edits.
    store
        .import_shared(transfer.id(), entry)
        .map_err(|e| e.to_string())?;
    // Private save and transfer acknowledgement are different contracts.
    // Never report acceptance while the private write can still be rejected.
    if !store.shared_import_confirmed(transfer.id()) {
        return Ok(false);
    }
    let receipt = transfer.send(TransferAction::Accept);
    let result = receipt.pending().clone();
    result.map(|()| true).map_err(|e| e.to_string())
}

/// Repair confirmed accepted copies after restart or a prior interrupted
/// import. Only current-recipient transfers qualify; a tombstone counts as an
/// intentional local deletion and is never resurrected. Call again on changes
/// so a temporarily conflicting store write can recover.
pub fn reconcile_received(
    ctx: &mut Context,
    store: &mut PasswordStore,
    transfers: &[Instance<LoginTransfer>],
) -> Result<(), String> {
    reconcile_into(&ctx.air, store, transfers)
}

fn reconcile_into(
    air: &Air,
    store: &mut PasswordStore,
    transfers: &[Instance<LoginTransfer>],
) -> Result<(), String> {
    for transfer in transfers {
        let Some(snapshot) = transfer.confirmed() else {
            continue;
        };
        if snapshot.recipient != air.me()
            || snapshot.status != TransferStatus::Accepted
            || snapshot.validate().is_err()
            || store.shared_import_confirmed(transfer.id())
        {
            continue;
        }
        store
            .import_shared(transfer.id(), snapshot.entry.clone())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn decline(ctx: &mut Context, transfer: &mut Instance<LoginTransfer>) -> Result<(), String> {
    if transfer.pending().recipient != ctx.air.me() {
        return Err(TransferError::Unauthorized.to_string());
    }
    let receipt = transfer.send(TransferAction::Decline);
    let result = receipt.pending().clone();
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::MemoryAir;
    fn names() -> (Name, Name) {
        (
            Name::orange_me(),
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
                .parse()
                .unwrap(),
        )
    }
    fn metadata(signer: Name) -> Metadata {
        Metadata {
            signer,
            timestamp: 1_000_000_000,
            confirmed: true,
        }
    }
    fn fixture() -> LoginTransfer {
        let (sender, recipient) = names();
        let mut entry = Entry::new();
        entry.title = "Example".into();
        entry.password = "private-test-value".into();
        LoginTransfer::init(
            LoginTransferInit {
                recipient,
                nonce: Id::random(),
                entry,
            },
            metadata(sender),
        )
    }
    #[test]
    fn only_recipient_can_accept_or_decline_and_payload_is_immutable() {
        let mut transfer = fixture();
        let original = transfer.entry.clone();
        assert_eq!(
            transfer.apply(TransferAction::Accept, metadata(transfer.sender)),
            Err(TransferError::Unauthorized)
        );
        transfer
            .apply(TransferAction::Accept, metadata(transfer.recipient))
            .unwrap();
        transfer
            .apply(TransferAction::Accept, metadata(transfer.recipient))
            .unwrap();
        assert_eq!(
            transfer.apply(TransferAction::Decline, metadata(transfer.recipient)),
            Err(TransferError::Closed)
        );
        assert_eq!(transfer.entry, original);
        let mut declined = fixture();
        declined
            .apply(TransferAction::Decline, metadata(declined.recipient))
            .unwrap();
        assert_eq!(
            declined.apply(TransferAction::Accept, metadata(declined.recipient)),
            Err(TransferError::Closed)
        );
    }
    #[test]
    fn malformed_names_payloads_and_debug_output_are_handled() {
        let (me, other) = names();
        assert_eq!(recipient(&format!("  {other}\n"), me).unwrap(), other);
        assert!(recipient(&me.to_string(), me).is_err());
        assert!(recipient("not-a-name", me).is_err());
        let mut transfer = fixture();
        assert!(!format!("{transfer:?}").contains("private-test-value"));
        transfer.entry.password.clear();
        assert_eq!(transfer.validate(), Err(TransferError::Invalid));
        transfer.entry.password = "password".into();
        transfer.entry.notes = "n".repeat(40_000);
        assert_eq!(transfer.validate(), Err(TransferError::Invalid));
    }

    fn incoming(
        status: TransferStatus,
    ) -> (MemoryAir, Air, PasswordStore, Instance<LoginTransfer>) {
        let memory = MemoryAir::default();
        let air = Air::new(memory.clone());
        let mut value = fixture();
        value.sender = names().1;
        value.status = status;
        let id = memory.received(value);
        let transfer = open(&air).list().remove(&id).unwrap();
        let store = PasswordStore::from_air(&air);
        (memory, air, store, transfer)
    }

    #[test]
    fn acceptance_waits_for_private_copy_confirmation() {
        let (memory, air, mut store, mut transfer) = incoming(TransferStatus::Pending);
        memory.confirm_private(false);
        assert!(!receive_into(&air, &mut store, &mut transfer).unwrap());
        assert_eq!(transfer.pending().status, TransferStatus::Pending);
        assert_eq!(store.entries().unwrap().len(), 1);
        memory.confirm_private(true);
        assert!(receive_into(&air, &mut store, &mut transfer).unwrap());
        assert_eq!(transfer.pending().status, TransferStatus::Accepted);
        assert_eq!(store.entries().unwrap().len(), 1);
    }

    #[test]
    fn accepted_copy_recovers_missing_store_without_overwriting_or_resurrecting() {
        let (_, air, mut store, transfer) = incoming(TransferStatus::Accepted);
        assert!(store.entries().unwrap().is_empty());
        reconcile_into(&air, &mut store, std::slice::from_ref(&transfer)).unwrap();
        let mut entry = store.entries().unwrap().remove(0);
        let id = entry.id.clone();
        entry.username = "recipient edit".into();
        store.save(entry).unwrap();
        reconcile_into(&air, &mut store, std::slice::from_ref(&transfer)).unwrap();
        assert_eq!(store.entries().unwrap()[0].username, "recipient edit");
        store.delete(&id).unwrap();
        reconcile_into(&air, &mut store, &[transfer]).unwrap();
        assert!(store.entries().unwrap().is_empty());
    }
}
