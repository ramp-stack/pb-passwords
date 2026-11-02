//! Keep request push subscriptions active independently of the visible page.
//! Ramp's notification payload is generic and never contains login contents.
use crate::requests::{LoginRequest, RequestStatus};
use prism::{
    air::{Air, Instance, Instances, Name},
    service::Service,
};

pub struct RequestNotifications(Instances<LoginRequest>, Air);

impl Service for RequestNotifications {
    type Init = ();
    type Predicate = Instance<LoginRequest>;

    async fn init(air: Air, _: ()) -> Self {
        let requests = air.instances::<LoginRequest>();
        for request in requests.list().into_values() {
            air.start::<RequestSubscription>(request);
        }
        Self(requests, air)
    }

    async fn predicate(&mut self) -> Self::Predicate {
        self.0.next().await
    }

    async fn handle(&mut self, request: Self::Predicate) {
        self.1.start::<RequestSubscription>(request);
    }
}

struct RequestSubscription {
    request: Instance<LoginRequest>,
    me: Name,
    enabled: bool,
}

impl Service for RequestSubscription {
    type Init = Instance<LoginRequest>;
    type Predicate = bool;

    async fn init(air: Air, request: Self::Init) -> Self {
        let me = air.me();
        let enabled = should_notify(&request.pending(), me);
        request.notify(enabled);
        Self {
            request,
            me,
            enabled,
        }
    }

    async fn predicate(&mut self) -> bool {
        self.request.next().await;
        should_notify(&self.request.pending(), self.me)
    }

    async fn handle(&mut self, enabled: bool) {
        if enabled != self.enabled {
            self.request.notify(enabled);
            self.enabled = enabled;
        }
    }
}

fn should_notify(request: &LoginRequest, me: Name) -> bool {
    if request.validate().is_err() {
        return false;
    }
    if request.requester == me {
        request.status != RequestStatus::Denied
            && !(request.status == RequestStatus::Responded && request.response_saved)
    } else {
        request.recipient == me
            && matches!(
                request.status,
                RequestStatus::Pending | RequestStatus::Accepted
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{requests::LoginRequestInit, storage::Entry};
    use prism::air::{Contract, Id, Metadata};

    #[test]
    fn only_participants_keep_active_request_notifications() {
        let requester = Name::orange_me();
        let recipient = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
            .parse()
            .unwrap();
        let stranger = "0379be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
            .parse()
            .unwrap();
        let mut request = LoginRequest::init(
            LoginRequestInit {
                recipient,
                subject: "Amazon".into(),
                nonce: Id::random(),
            },
            Metadata {
                signer: requester,
                timestamp: 0,
                confirmed: true,
            },
        );
        for state in [RequestStatus::Pending, RequestStatus::Accepted] {
            request.status = state;
            assert!(should_notify(&request, requester));
            assert!(should_notify(&request, recipient));
            assert!(!should_notify(&request, stranger));
        }
        request.status = RequestStatus::Denied;
        assert!(!should_notify(&request, requester));
        assert!(!should_notify(&request, recipient));

        let mut entry = Entry::new();
        entry.title = "Amazon".into();
        entry.password = "test-only credential".into();
        request.responses.push(entry);
        request.status = RequestStatus::Responded;
        assert!(should_notify(&request, requester));
        assert!(!should_notify(&request, recipient));
        request.response_saved = true;
        assert!(!should_notify(&request, requester));

        request.status = RequestStatus::Pending;
        assert!(
            !should_notify(&request, requester),
            "reject invalid request state"
        );
    }
}
