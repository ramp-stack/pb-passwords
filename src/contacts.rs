//! Saved contact names merged with AIR sharing history, without duplicating credentials.
use crate::requests::{LoginRequest, RequestStatus};
use crate::sharing::LoginTransfer;
use chk::profiles::Profile;
use prism::air::{Instance, Name};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Contact {
    pub name: Name,
    pub last_shared_at: u64,
    pub share_count: usize,
}

/// Includes queued, accepted, and declined outgoing copies: each represents an
/// explicit share with that person. Incoming copies never add unsolicited contacts.
pub(crate) fn from_transfers(me: Name, transfers: &[Instance<LoginTransfer>]) -> Vec<Contact> {
    let mut contacts = BTreeMap::new();
    let mut seen = HashSet::new();
    for instance in transfers {
        if seen.insert(instance.id()) {
            record(me, &instance.pending(), &mut contacts);
        }
    }
    sorted(contacts)
}

/// Profiles are the same locally saved contact records used by Orange. Merge
/// them with sharing history so adding a contact never requires sending a login.
pub(crate) fn from_sources(
    me: Name,
    transfers: &[Instance<LoginTransfer>],
    profiles: &[Instance<Profile>],
) -> Vec<Contact> {
    let mut contacts: BTreeMap<_, _> = from_transfers(me, transfers)
        .into_iter()
        .map(|contact| (contact.name, contact))
        .collect();
    for profile in profiles {
        let Some(name) = profile.pending().name.filter(|name| *name != me) else {
            continue;
        };
        contacts.entry(name).or_insert(Contact {
            name,
            last_shared_at: 0,
            share_count: 0,
        });
    }
    sorted(contacts)
}

fn record(me: Name, transfer: &LoginTransfer, contacts: &mut BTreeMap<Name, Contact>) {
    if transfer.sender != me || transfer.recipient == me || transfer.validate().is_err() {
        return;
    }
    let contact = contacts.entry(transfer.recipient).or_insert(Contact {
        name: transfer.recipient,
        last_shared_at: transfer.created_at,
        share_count: 0,
    });
    contact.last_shared_at = contact.last_shared_at.max(transfer.created_at);
    contact.share_count += 1;
}

/// Responding to a request is an explicit share too. Incoming unanswered requests
/// never become contacts, and repeated snapshots cannot inflate the share count.
pub(crate) fn with_requests(
    me: Name,
    transfers: &[Instance<LoginTransfer>],
    profiles: &[Instance<Profile>],
    requests: &[Instance<LoginRequest>],
) -> Vec<Contact> {
    let mut contacts: BTreeMap<_, _> = from_sources(me, transfers, profiles)
        .into_iter()
        .map(|contact| (contact.name, contact))
        .collect();
    let mut seen = HashSet::new();
    for instance in requests {
        if !seen.insert(instance.id()) {
            continue;
        }
        let Some(request) = instance.confirmed() else {
            continue;
        };
        if request.recipient != me
            || request.status != RequestStatus::Responded
            || request.validate().is_err()
        {
            continue;
        }
        let contact = contacts.entry(request.requester).or_insert(Contact {
            name: request.requester,
            last_shared_at: 0,
            share_count: 0,
        });
        contact.last_shared_at = contact.last_shared_at.max(request.created_at);
        contact.share_count += request.responses.len();
    }
    sorted(contacts)
}

fn sorted(contacts: BTreeMap<Name, Contact>) -> Vec<Contact> {
    let mut contacts: Vec<_> = contacts.into_values().collect();
    contacts.sort_by(|a, b| {
        b.last_shared_at
            .cmp(&a.last_shared_at)
            .then_with(|| a.name.cmp(&b.name))
    });
    contacts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        sharing::{LoginTransferInit, TransferStatus},
        storage::{Entry, tests::MemoryAir},
    };
    use prism::air::{Air, Id};

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

    fn transfer(sender: Name, recipient: Name, created_at: u64) -> LoginTransfer {
        let mut entry = Entry::new();
        entry.title = "Private service".into();
        entry.username = "private username".into();
        entry.password = "private password".into();
        entry.notes = "private notes".into();
        LoginTransfer {
            sender,
            recipient,
            entry,
            status: TransferStatus::Pending,
            created_at,
        }
    }

    fn summarize(me: Name, transfers: &[LoginTransfer]) -> Vec<Contact> {
        let mut contacts = BTreeMap::new();
        for transfer in transfers {
            record(me, transfer, &mut contacts);
        }
        sorted(contacts)
    }

    #[test]
    fn repeated_shares_group_one_contact_across_delivery_states() {
        let (me, recipient, _) = names();
        let mut accepted = transfer(me, recipient, 20);
        accepted.status = TransferStatus::Accepted;
        let mut declined = transfer(me, recipient, 30);
        declined.status = TransferStatus::Declined;
        let contacts = summarize(me, &[transfer(me, recipient, 10), declined, accepted]);
        assert_eq!(
            contacts,
            vec![Contact {
                name: recipient,
                last_shared_at: 30,
                share_count: 3,
            }]
        );
        let debug = format!("{contacts:?}");
        for value in [
            "Private service",
            "private username",
            "private password",
            "private notes",
        ] {
            assert!(!debug.contains(value));
        }
    }

    #[test]
    fn incoming_third_party_self_and_invalid_copies_are_excluded() {
        let (me, first, second) = names();
        let mut invalid = transfer(me, first, 100);
        invalid.entry.password.clear();
        let contacts = summarize(
            me,
            &[
                transfer(first, me, 200),
                transfer(first, second, 300),
                transfer(me, me, 400),
                invalid,
                transfer(me, second, 50),
            ],
        );
        assert_eq!(
            contacts,
            vec![Contact {
                name: second,
                last_shared_at: 50,
                share_count: 1
            }]
        );
    }

    #[test]
    fn latest_share_sorts_first_and_ties_are_stable() {
        let (me, first, second) = names();
        let contacts = summarize(
            me,
            &[
                transfer(me, first, 10),
                transfer(me, second, 20),
                transfer(me, first, 30),
            ],
        );
        assert_eq!(
            contacts
                .iter()
                .map(|contact| contact.name)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
        let forward = summarize(me, &[transfer(me, first, 20), transfer(me, second, 20)]);
        let reverse = summarize(me, &[transfer(me, second, 20), transfer(me, first, 20)]);
        assert_eq!(forward, reverse);
    }

    #[test]
    fn reopening_air_history_restores_contacts_without_double_counting_instances() {
        let memory = MemoryAir::default();
        let air = Air::new(memory.clone());
        let (me, recipient, _) = names();
        let snapshot = transfer(me, recipient, 0);
        let sent = air.instances::<LoginTransfer>().create(LoginTransferInit {
            recipient,
            nonce: Id::random(),
            entry: snapshot.entry.clone(),
        });
        assert_eq!(from_transfers(me, &[sent.clone(), sent])[0].share_count, 1);
        let reopened = Air::new(memory);
        let restored: Vec<_> = reopened
            .instances::<LoginTransfer>()
            .list()
            .into_values()
            .collect();
        assert_eq!(
            from_transfers(me, &restored),
            vec![Contact {
                name: recipient,
                last_shared_at: 0,
                share_count: 1,
            }]
        );
    }

    #[test]
    fn answered_requests_count_shared_logins_once_and_ignore_unanswered_contacts() {
        let memory = MemoryAir::default();
        let (me, requester, unanswered_requester) = names();
        let mut second_login = transfer(me, requester, 0).entry.clone();
        second_login.title = "Another private service".into();
        let answered = LoginRequest {
            requester,
            recipient: me,
            subject: "Requested logins".into(),
            status: RequestStatus::Responded,
            created_at: 20,
            responses: vec![transfer(me, requester, 0).entry.clone(), second_login],
            response_saved: false,
        };
        assert!(answered.validate().is_ok());
        let answered_id = memory.request(answered);
        for status in [RequestStatus::Pending, RequestStatus::Accepted] {
            memory.request(LoginRequest {
                requester: unanswered_requester,
                recipient: me,
                subject: "Unanswered request".into(),
                status,
                created_at: 30,
                responses: vec![],
                response_saved: false,
            });
        }
        let air = Air::new(memory.clone());
        let mut requests: Vec<_> = crate::requests::open(&air).list().into_values().collect();
        let repeated = requests
            .iter()
            .find(|request| request.id() == answered_id)
            .unwrap()
            .clone();
        requests.push(repeated);
        let expected = vec![Contact {
            name: requester,
            last_shared_at: 20,
            share_count: 2,
        }];
        assert_eq!(with_requests(me, &[], &[], &requests), expected);

        let reopened = Air::new(memory);
        let restored: Vec<_> = crate::requests::open(&reopened)
            .list()
            .into_values()
            .collect();
        assert_eq!(with_requests(me, &[], &[], &restored), expected);
    }

    #[test]
    fn saved_contacts_restore_without_a_transfer_and_merge_by_name() {
        let memory = MemoryAir::default();
        let air = Air::new(memory.clone());
        let (me, first, second) = names();
        // Separate records for one name still produce one contact row.
        let profiles = air.instances::<Profile>();
        profiles.create(first);
        profiles.create(first);
        profiles.create(second);
        assert!(air.instances::<LoginTransfer>().list().is_empty());
        let saved: Vec<_> = profiles.list().into_values().collect();
        let contacts = from_sources(me, &[], &saved);
        assert_eq!(contacts.len(), 2);
        assert!(contacts.iter().all(|contact| contact.share_count == 0));

        let reopened = Air::new(memory);
        let restored: Vec<_> = reopened
            .instances::<Profile>()
            .list()
            .into_values()
            .collect();
        assert_eq!(from_sources(me, &[], &restored), contacts);
        let entry = transfer(me, first, 0).entry.clone();
        let sent = reopened
            .instances::<LoginTransfer>()
            .create(LoginTransferInit {
                recipient: first,
                nonce: Id::random(),
                entry,
            });
        let merged = from_sources(me, &[sent], &restored);
        assert_eq!(merged.len(), 2);
        assert_eq!(
            merged
                .iter()
                .find(|contact| contact.name == first)
                .unwrap()
                .share_count,
            1
        );
        assert_eq!(
            merged
                .iter()
                .find(|contact| contact.name == second)
                .unwrap()
                .share_count,
            0
        );
    }

    #[test]
    fn profile_sources_ignore_self_and_missing_names() {
        use prism::air::{Contract, Metadata};
        let memory = MemoryAir::default();
        let (me, first, _) = names();
        let metadata = || Metadata {
            signer: me,
            timestamp: 1,
            confirmed: true,
        };
        memory.profile(Profile::init(me, metadata()));
        let mut unnamed = Profile::init(first, metadata());
        unnamed.name = None;
        memory.profile(unnamed);
        memory.profile(Profile::init(first, metadata()));
        let air = Air::new(memory);
        let profiles: Vec<_> = air.instances::<Profile>().list().into_values().collect();
        assert_eq!(
            from_sources(me, &[], &profiles),
            vec![Contact {
                name: first,
                last_shared_at: 0,
                share_count: 0,
            }]
        );
    }
}
