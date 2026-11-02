//! A password request and its explicitly selected response share one AIR contract.
//! The response is committed as one immutable batch; private imports are retry-safe.
use crate::storage::{Entry, PasswordStore};
use chk::Context;
use prism::air::{Air, Contract, Id, Instance, Instances, Metadata, Name};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_RESPONSES: usize = 32;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RequestStatus {
    Pending,
    Accepted,
    Denied,
    Responded,
}

#[derive(Clone, Hash, Serialize, Deserialize)]
pub struct LoginRequestInit {
    pub recipient: Name,
    pub subject: String,
    pub nonce: Id,
}

impl fmt::Debug for LoginRequestInit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginRequestInit")
            .field("recipient", &self.recipient)
            .field("subject", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub requester: Name,
    pub recipient: Name,
    pub subject: String,
    pub status: RequestStatus,
    pub created_at: u64,
    pub responses: Vec<Entry>,
    pub response_saved: bool,
}

impl fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginRequest")
            .field("requester", &self.requester)
            .field("recipient", &self.recipient)
            .field("status", &self.status)
            .field("subject", &"[REDACTED]")
            .field("responses", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Hash, Serialize, Deserialize)]
pub enum RequestAction {
    Accept,
    Deny,
    Respond(Vec<Entry>),
    MarkReceived,
}

impl fmt::Debug for RequestAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept => f.write_str("Accept"),
            Self::Deny => f.write_str("Deny"),
            Self::Respond(entries) => f.debug_tuple("Respond").field(&entries.len()).finish(),
            Self::MarkReceived => f.write_str("MarkReceived"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestError {
    Unauthorized,
    Invalid,
    Closed,
    NotAccepted,
    EmptyResponse,
    TooManyResponses,
    ResponseTooLarge,
    Unavailable,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unauthorized => "Only the intended person can handle this request.",
            Self::Invalid => "This password request is invalid.",
            Self::Closed => "This request has already been handled.",
            Self::NotAccepted => "Accept this request before choosing passwords.",
            Self::EmptyResponse => "Choose at least one login to share.",
            Self::TooManyResponses => "Choose up to 32 logins at a time.",
            Self::ResponseTooLarge => "These logins are too large to send together. Choose fewer.",
            Self::Unavailable => "This password request is not available yet.",
        })
    }
}

impl Contract for LoginRequest {
    type Init = LoginRequestInit;
    type Message = RequestAction;
    type Result = Result<(), RequestError>;

    fn id() -> Id {
        Id::hash("org.ramp.peanut-butter-passwords.LoginRequest.v1")
    }

    fn init(init: Self::Init, metadata: Metadata) -> Self {
        Self {
            requester: metadata.signer,
            recipient: init.recipient,
            subject: init.subject,
            status: RequestStatus::Pending,
            created_at: metadata.timestamp / 1_000_000_000,
            responses: vec![],
            response_saved: false,
        }
    }

    fn apply(&mut self, action: Self::Message, metadata: Metadata) -> Self::Result {
        self.validate()?;
        if matches!(action, RequestAction::MarkReceived) {
            if metadata.signer != self.requester {
                return Err(RequestError::Unauthorized);
            }
            if self.status != RequestStatus::Responded {
                return Err(RequestError::Closed);
            }
            self.response_saved = true;
            return Ok(());
        }
        if metadata.signer != self.recipient {
            return Err(RequestError::Unauthorized);
        }
        match action {
            RequestAction::Accept => match self.status {
                RequestStatus::Pending | RequestStatus::Accepted => {
                    self.status = RequestStatus::Accepted;
                    Ok(())
                }
                _ => Err(RequestError::Closed),
            },
            RequestAction::Deny => match self.status {
                RequestStatus::Pending | RequestStatus::Accepted | RequestStatus::Denied => {
                    self.status = RequestStatus::Denied;
                    Ok(())
                }
                RequestStatus::Responded => Err(RequestError::Closed),
            },
            RequestAction::Respond(entries) => {
                validate_response(&entries)?;
                if self.status == RequestStatus::Responded && self.responses == entries {
                    return Ok(());
                }
                match self.status {
                    RequestStatus::Accepted => {
                        self.responses = entries;
                        self.status = RequestStatus::Responded;
                        Ok(())
                    }
                    RequestStatus::Pending => Err(RequestError::NotAccepted),
                    _ => Err(RequestError::Closed),
                }
            }
            RequestAction::MarkReceived => unreachable!(),
        }
    }
}

impl LoginRequest {
    pub fn validate(&self) -> Result<(), RequestError> {
        if self.requester == self.recipient
            || validate_subject(&self.subject).as_deref() != Ok(self.subject.as_str())
        {
            return Err(RequestError::Invalid);
        }
        if self.status == RequestStatus::Responded {
            validate_response(&self.responses)?;
        } else if !self.responses.is_empty() || self.response_saved {
            return Err(RequestError::Invalid);
        }
        Ok(())
    }
}

pub fn validate_subject(subject: &str) -> Result<String, String> {
    let subject = subject.trim();
    if subject.is_empty() {
        Err("Enter the app or website you’re asking for.".into())
    } else if subject.len() > 256 {
        Err("This request is too long. Use a shorter app or website name.".into())
    } else {
        Ok(subject.to_owned())
    }
}

fn validate_response(entries: &[Entry]) -> Result<(), RequestError> {
    if entries.is_empty() {
        return Err(RequestError::EmptyResponse);
    }
    if entries.len() > MAX_RESPONSES {
        return Err(RequestError::TooManyResponses);
    }
    let mut total = 0;
    for entry in entries {
        if entry.validate().is_err()
            || !entry.id.is_empty()
            || entry.revision != 0
            || entry.updated_at != 0
        {
            return Err(RequestError::Invalid);
        }
        total += entry.title.len()
            + entry.username.len()
            + entry.password.len()
            + entry.url.len()
            + entry.notes.len();
    }
    if total > MAX_RESPONSE_BYTES {
        Err(RequestError::ResponseTooLarge)
    } else {
        Ok(())
    }
}

pub fn open(air: &Air) -> Instances<LoginRequest> {
    air.instances::<LoginRequest>()
}

pub fn send_request(
    ctx: &mut Context,
    recipient: Name,
    subject: String,
) -> Result<Instance<LoginRequest>, String> {
    send_into(&ctx.air, recipient, subject)
}

fn send_into(
    air: &Air,
    recipient: Name,
    subject: String,
) -> Result<Instance<LoginRequest>, String> {
    let subject = validate_subject(&subject)?;
    if recipient == air.me() {
        return Err("Choose another person’s Peanut Butter Name.".into());
    }
    let request = open(air).create(LoginRequestInit {
        recipient,
        subject,
        nonce: Id::random(),
    });
    request.notify(true);
    request.share(recipient);
    Ok(request)
}

fn act(
    air: &Air,
    request: &mut Instance<LoginRequest>,
    action: RequestAction,
) -> Result<(), String> {
    let snapshot = request.pending();
    snapshot.validate().map_err(|e| e.to_string())?;
    let actor = if matches!(action, RequestAction::MarkReceived) {
        snapshot.requester
    } else {
        snapshot.recipient
    };
    if air.me() != actor {
        return Err(RequestError::Unauthorized.to_string());
    }
    // Watcher refreshes can repeat a completed save or resume an already
    // accepted request. Do not enqueue another identical AIR write each time.
    let unchanged = match &action {
        RequestAction::Accept => snapshot.status == RequestStatus::Accepted,
        RequestAction::Deny => snapshot.status == RequestStatus::Denied,
        RequestAction::Respond(entries) => {
            snapshot.status == RequestStatus::Responded && snapshot.responses == *entries
        }
        RequestAction::MarkReceived => {
            snapshot.status == RequestStatus::Responded && snapshot.response_saved
        }
    };
    drop(snapshot);
    if request.confirmed().is_none() {
        return Err(RequestError::Unavailable.to_string());
    }
    if unchanged {
        return Ok(());
    }
    let receipt = request.send(action);
    let result = *receipt.pending();
    result.map_err(|e| e.to_string())
}

pub fn accept(ctx: &mut Context, request: &mut Instance<LoginRequest>) -> Result<(), String> {
    act(&ctx.air, request, RequestAction::Accept)
}

pub fn deny(ctx: &mut Context, request: &mut Instance<LoginRequest>) -> Result<(), String> {
    act(&ctx.air, request, RequestAction::Deny)
}

pub fn respond(
    ctx: &mut Context,
    request: &mut Instance<LoginRequest>,
    entries: Vec<Entry>,
) -> Result<(), String> {
    respond_into(&ctx.air, request, entries)
}

fn respond_into(
    air: &Air,
    request: &mut Instance<LoginRequest>,
    mut entries: Vec<Entry>,
) -> Result<(), String> {
    for entry in &mut entries {
        entry.id.clear();
        entry.revision = 0;
        entry.updated_at = 0;
    }
    validate_response(&entries).map_err(|e| e.to_string())?;
    act(air, request, RequestAction::Respond(entries))
}

pub fn response_id(request: Id, index: usize) -> Id {
    Id::hash(&(
        "peanut-butter-passwords.request-response.v1",
        request,
        index as u64,
    ))
}

pub fn receive_response(
    ctx: &mut Context,
    store: &mut PasswordStore,
    request: &mut Instance<LoginRequest>,
) -> Result<bool, String> {
    receive_into(&ctx.air, store, request)
}

fn receive_into(
    air: &Air,
    store: &mut PasswordStore,
    request: &mut Instance<LoginRequest>,
) -> Result<bool, String> {
    let snapshot = request
        .confirmed()
        .ok_or_else(|| RequestError::Unavailable.to_string())?;
    snapshot.validate().map_err(|e| e.to_string())?;
    if snapshot.requester != air.me() {
        return Err(RequestError::Unauthorized.to_string());
    }
    if snapshot.status != RequestStatus::Responded {
        return Err(RequestError::Closed.to_string());
    }
    let entries = snapshot.responses.clone();
    drop(snapshot);
    for (index, entry) in entries.iter().enumerate() {
        store
            .import_shared(response_id(request.id(), index), entry.clone())
            .map_err(|e| e.to_string())?;
    }
    if !(0..entries.len())
        .all(|index| store.shared_import_confirmed(response_id(request.id(), index)))
    {
        return Ok(false);
    }
    act(air, request, RequestAction::MarkReceived)?;
    Ok(true)
}

/// Recover only responses the requester previously chose to save. Unreviewed
/// responses are never imported automatically; local edits/deletions survive.
pub fn reconcile_received(
    ctx: &mut Context,
    store: &mut PasswordStore,
    requests: &[Instance<LoginRequest>],
) -> Result<(), String> {
    reconcile_into(&ctx.air, store, requests)
}

fn reconcile_into(
    air: &Air,
    store: &mut PasswordStore,
    requests: &[Instance<LoginRequest>],
) -> Result<(), String> {
    for request in requests {
        let Some(snapshot) = request.confirmed() else {
            continue;
        };
        if snapshot.requester != air.me()
            || !snapshot.response_saved
            || snapshot.status != RequestStatus::Responded
            || snapshot.validate().is_err()
        {
            continue;
        }
        for (index, entry) in snapshot.responses.iter().enumerate() {
            let id = response_id(request.id(), index);
            if !store.shared_import_confirmed(id) {
                store
                    .import_shared(id, entry.clone())
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::MemoryAir;

    fn names() -> (Name, Name, Name) {
        (
            Name::orange_me(),
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
                .parse()
                .unwrap(),
            "0379be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
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
    fn fixture() -> LoginRequest {
        let (requester, recipient, _) = names();
        LoginRequest::init(
            LoginRequestInit {
                recipient,
                subject: "Amazon family account".into(),
                nonce: Id::random(),
            },
            metadata(requester),
        )
    }
    fn entry(name: &str) -> Entry {
        let mut entry = Entry::new();
        entry.title = name.into();
        entry.username = "family@example.test".into();
        entry.password = "private request response password".into();
        entry.notes = "private request response note".into();
        entry
    }
    fn responded() -> LoginRequest {
        let mut request = fixture();
        request
            .apply(RequestAction::Accept, metadata(request.recipient))
            .unwrap();
        request
            .apply(
                RequestAction::Respond(vec![entry("Amazon"), entry("Amazon shopping")]),
                metadata(request.recipient),
            )
            .unwrap();
        request
    }

    #[test]
    fn only_recipient_can_accept_deny_or_send_and_only_requester_can_acknowledge() {
        let mut request = fixture();
        for actor in [request.requester, names().2] {
            for action in [
                RequestAction::Accept,
                RequestAction::Deny,
                RequestAction::Respond(vec![entry("Amazon")]),
            ] {
                assert_eq!(
                    request.apply(action, metadata(actor)),
                    Err(RequestError::Unauthorized)
                );
            }
        }
        assert_eq!(request.status, RequestStatus::Pending);
        assert!(request.responses.is_empty());
        assert_eq!(
            request.apply(RequestAction::MarkReceived, metadata(request.requester)),
            Err(RequestError::Closed)
        );
        let mut request = responded();
        for actor in [request.recipient, names().2] {
            assert_eq!(
                request.apply(RequestAction::MarkReceived, metadata(actor)),
                Err(RequestError::Unauthorized)
            );
        }
        assert!(!request.response_saved);
        request
            .apply(RequestAction::MarkReceived, metadata(request.requester))
            .unwrap();
        request
            .apply(RequestAction::MarkReceived, metadata(request.requester))
            .unwrap();
        assert!(request.response_saved);
    }

    #[test]
    fn acceptance_reveals_nothing_and_denial_is_terminal() {
        let mut request = fixture();
        assert_eq!(
            request.apply(
                RequestAction::Respond(vec![entry("Amazon")]),
                metadata(request.recipient)
            ),
            Err(RequestError::NotAccepted)
        );
        request
            .apply(RequestAction::Accept, metadata(request.recipient))
            .unwrap();
        request
            .apply(RequestAction::Accept, metadata(request.recipient))
            .unwrap();
        assert_eq!(request.status, RequestStatus::Accepted);
        assert!(request.responses.is_empty());
        let bytes = serde_json::to_vec(&request).unwrap();
        let mut reopened: LoginRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(reopened.status, RequestStatus::Accepted);
        reopened
            .apply(RequestAction::Deny, metadata(request.recipient))
            .unwrap();
        reopened
            .apply(RequestAction::Deny, metadata(request.recipient))
            .unwrap();
        assert_eq!(
            reopened.apply(RequestAction::Accept, metadata(request.recipient)),
            Err(RequestError::Closed)
        );
        assert_eq!(
            reopened.apply(
                RequestAction::Respond(vec![entry("Amazon")]),
                metadata(request.recipient)
            ),
            Err(RequestError::Closed)
        );
        assert!(reopened.responses.is_empty());
    }

    #[test]
    fn response_is_one_immutable_batch_and_retries_are_idempotent() {
        let mut request = fixture();
        request
            .apply(RequestAction::Accept, metadata(request.recipient))
            .unwrap();
        let good = entry("Amazon");
        let mut bad = entry("Broken");
        bad.password.clear();
        assert_eq!(
            request.apply(
                RequestAction::Respond(vec![good.clone(), bad]),
                metadata(request.recipient)
            ),
            Err(RequestError::Invalid)
        );
        assert_eq!(request.status, RequestStatus::Accepted);
        assert!(request.responses.is_empty());
        let responses = vec![good, entry("Family shopping")];
        request
            .apply(
                RequestAction::Respond(responses.clone()),
                metadata(request.recipient),
            )
            .unwrap();
        request
            .apply(
                RequestAction::Respond(responses.clone()),
                metadata(request.recipient),
            )
            .unwrap();
        assert_eq!(request.responses, responses);
        assert_eq!(
            request.apply(
                RequestAction::Respond(vec![entry("Replacement")]),
                metadata(request.recipient)
            ),
            Err(RequestError::Closed)
        );
        assert_eq!(
            request.apply(RequestAction::Deny, metadata(request.recipient)),
            Err(RequestError::Closed)
        );
        assert_eq!(request.responses, responses);
    }

    #[test]
    fn response_validation_rejects_bookkeeping_and_oversized_batches() {
        assert_eq!(validate_response(&[]), Err(RequestError::EmptyResponse));
        assert_eq!(
            validate_response(&vec![entry("Amazon"); MAX_RESPONSES + 1]),
            Err(RequestError::TooManyResponses)
        );
        // Two distinct saved logins may have identical fields. Their response
        // positions still give them distinct, stable import identities.
        assert!(validate_response(&[entry("Amazon"), entry("Amazon")]).is_ok());
        for (id, revision, updated_at) in [("local-secret-id", 0, 0), ("", 1, 0), ("", 0, 1)] {
            let mut value = entry("Amazon");
            value.id = id.into();
            value.revision = revision;
            value.updated_at = updated_at;
            assert_eq!(validate_response(&[value]), Err(RequestError::Invalid));
        }
        let entries: Vec<_> = (0..MAX_RESPONSES)
            .map(|i| {
                let mut value = entry(&format!("Account {i}"));
                value.notes = "n".repeat(20_000);
                value
            })
            .collect();
        assert_eq!(
            validate_response(&entries),
            Err(RequestError::ResponseTooLarge)
        );
    }

    #[test]
    fn send_validates_subject_and_self_and_arms_response_notifications() {
        let memory = MemoryAir::default();
        let air = Air::new(memory.clone());
        assert!(send_into(&air, names().1, "   ".into()).is_err());
        assert!(send_into(&air, names().1, "x".repeat(257)).is_err());
        assert!(send_into(&air, air.me(), "Amazon".into()).is_err());
        assert!(open(&air).list().is_empty());
        let request = send_into(&air, names().1, "  Amazon  \n".into()).unwrap();
        assert_eq!(request.pending().subject, "Amazon");
        assert_eq!(request.pending().requester, air.me());
        assert!(request.pending().responses.is_empty());
        assert_eq!(memory.request_notifications(request.id()), Some(true));
        let reopened = Air::new(memory);
        let request = open(&reopened).list().remove(&request.id()).unwrap();
        assert_eq!(request.pending().status, RequestStatus::Pending);
    }

    #[test]
    fn respond_strips_source_ids_without_mutating_saved_entries() {
        let memory = MemoryAir::default();
        let air = Air::new(memory.clone());
        let mut snapshot = fixture();
        (snapshot.requester, snapshot.recipient) = (names().1, air.me());
        snapshot.status = RequestStatus::Accepted;
        let id = memory.request(snapshot);
        let mut request = open(&air).list().remove(&id).unwrap();
        let mut source = entry("Amazon");
        source.id = "0123456789abcdef0123456789abcdef".into();
        source.revision = 7;
        source.updated_at = 12345;
        respond_into(&air, &mut request, vec![source.clone(), entry("Shopping")]).unwrap();
        assert_eq!(source.revision, 7);
        assert_eq!(source.updated_at, 12345);
        let received = request.pending();
        assert_eq!(received.status, RequestStatus::Responded);
        assert_eq!(received.responses.len(), 2);
        for value in &received.responses {
            assert!(value.id.is_empty());
            assert_eq!(value.revision, 0);
            assert_eq!(value.updated_at, 0);
        }
    }

    #[test]
    fn optimistic_response_can_roll_back_and_retry_before_confirmation() {
        let memory = MemoryAir::default();
        let air = Air::new(memory.clone());
        let mut incoming = fixture();
        (incoming.requester, incoming.recipient) = (names().1, names().0);
        incoming
            .apply(RequestAction::Accept, metadata(incoming.recipient))
            .unwrap();
        let id = memory.request(incoming);
        let mut request = open(&air).list().remove(&id).unwrap();

        memory.hold_request_confirmation(id);
        respond_into(&air, &mut request, vec![entry("Amazon")]).unwrap();
        assert_eq!(request.pending().status, RequestStatus::Responded);
        assert_eq!(request.confirmed().unwrap().status, RequestStatus::Accepted);
        assert!(request.confirmed().unwrap().responses.is_empty());

        memory.rollback_request(id);
        assert_eq!(request.pending().status, RequestStatus::Accepted);
        assert!(request.pending().responses.is_empty());
        assert_eq!(*request.pending(), *request.confirmed().unwrap());

        memory.hold_request_confirmation(id);
        respond_into(&air, &mut request, vec![entry("Amazon")]).unwrap();
        memory.confirm_request(id);
        assert_eq!(
            request.confirmed().unwrap().status,
            RequestStatus::Responded
        );
        assert_eq!(request.confirmed().unwrap().responses.len(), 1);
        assert_eq!(*request.pending(), *request.confirmed().unwrap());
    }

    #[test]
    fn save_all_waits_for_private_confirmation_and_survives_reopen_without_duplicates() {
        let memory = MemoryAir::default();
        let air = Air::new(memory.clone());
        let id = memory.request(responded());
        let mut request = open(&air).list().remove(&id).unwrap();
        let mut store = PasswordStore::from_air(&air);
        assert!(
            store.entries().unwrap().is_empty(),
            "responses require explicit saving"
        );
        memory.confirm_private(false);
        assert!(!receive_into(&air, &mut store, &mut request).unwrap());
        assert!(!request.pending().response_saved);
        assert_eq!(store.entries().unwrap().len(), 2);
        memory.confirm_private(true);
        assert!(receive_into(&air, &mut store, &mut request).unwrap());
        assert!(request.pending().response_saved);
        let mut saved = store.entries().unwrap();
        let mut edited = saved.remove(0);
        edited.username = "edited on requester device".into();
        store.save(edited).unwrap();
        store.delete(&saved[0].id).unwrap();
        drop(store);
        drop(air);
        let air = Air::new(memory);
        let mut store = PasswordStore::from_air(&air);
        let mut request = open(&air).list().remove(&id).unwrap();
        assert!(receive_into(&air, &mut store, &mut request).unwrap());
        let saved = store.entries().unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].username, "edited on requester device");
        assert_ne!(response_id(id, 0), response_id(id, 1));
        assert_ne!(response_id(id, 0), response_id(Id::random(), 0));
    }

    #[test]
    fn response_import_rejects_other_requesters_and_unanswered_requests() {
        let memory = MemoryAir::default();
        let air = Air::new(memory.clone());
        let mut other = responded();
        (other.requester, other.recipient) = (names().1, names().2);
        let other = memory.request(other);
        let pending = memory.request(fixture());
        let mut store = PasswordStore::from_air(&air);
        let mut requests = open(&air).list();
        for id in [other, pending] {
            assert!(receive_into(&air, &mut store, requests.get_mut(&id).unwrap()).is_err());
            assert!(store.entries().unwrap().is_empty());
            assert!(!requests[&id].pending().response_saved);
        }
    }

    #[test]
    fn reconciliation_only_restores_explicitly_saved_response_batches() {
        let memory = MemoryAir::default();
        let air = Air::new(memory.clone());
        let unread = memory.request(responded());
        let mut saved = responded();
        saved.response_saved = true;
        let saved = memory.request(saved);
        let requests: Vec<_> = open(&air).list().into_values().collect();
        let mut store = PasswordStore::from_air(&air);
        reconcile_into(&air, &mut store, &requests).unwrap();
        assert_eq!(store.entries().unwrap().len(), 2);
        assert!(!store.shared_import_confirmed(response_id(unread, 0)));
        assert!(store.shared_import_confirmed(response_id(saved, 0)));
        let mut entries = store.entries().unwrap();
        let mut edited = entries.remove(0);
        edited.username = "keep this local edit".into();
        store.save(edited).unwrap();
        store.delete(&entries[0].id).unwrap();
        reconcile_into(&air, &mut store, &requests).unwrap();
        let entries = store.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].username, "keep this local edit");
    }

    #[test]
    fn malformed_requests_and_debug_output_never_expose_request_contents() {
        let request = responded();
        let debug = format!(
            "{request:?} {:?} {:?}",
            RequestAction::Respond(request.responses.clone()),
            LoginRequestInit {
                recipient: names().1,
                subject: request.subject.clone(),
                nonce: Id::random()
            }
        );
        for secret in [
            &request.subject,
            &request.responses[0].password,
            &request.responses[0].notes,
        ] {
            assert!(!debug.contains(secret));
        }
        let mut invalid = fixture();
        invalid.requester = invalid.recipient;
        assert_eq!(invalid.validate(), Err(RequestError::Invalid));
        let mut invalid = fixture();
        invalid.subject = " untrimmed ".into();
        assert_eq!(invalid.validate(), Err(RequestError::Invalid));
        let mut invalid = fixture();
        invalid.responses = vec![entry("Unsolicited payload")];
        assert_eq!(invalid.validate(), Err(RequestError::Invalid));
        let mut invalid = fixture();
        invalid.response_saved = true;
        assert_eq!(invalid.validate(), Err(RequestError::Invalid));
    }
}
