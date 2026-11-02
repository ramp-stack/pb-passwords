//! Password requests use standard CHK forms, data items, and actions.
use super::*;
use std::collections::BTreeSet;

impl PasswordsApp {
    pub(super) fn requests_page(&self) -> Page {
        let app = self.clone();
        Page::watching(
            (
                (
                    Watch::list::<LoginRequest>(),
                    Watch::list::<LoginTransfer>(),
                ),
                Watch::list::<Profile>(),
                Watch::contract(self.session.borrow().instance()),
                self.changes.clone(),
            ),
            move |ctx: &mut Context, theme: &Theme, ((requests, transfers), profiles, _, ())| {
                let mut items = vec![];
                if let Err(error) =
                    requests::reconcile_received(ctx, &mut app.session.borrow_mut(), &requests)
                {
                    items.push(Display::instructions(&error));
                }
                if let Err(error) =
                    sharing::reconcile_received(ctx, &mut app.session.borrow_mut(), &transfers)
                {
                    items.push(Display::instructions(&error));
                }
                let received = transfers
                    .into_iter()
                    .filter_map(|instance| {
                        let transfer = instance.confirmed()?;
                        if transfer.recipient != app.name
                            || transfer.status != TransferStatus::Pending
                            || transfer.validate().is_err()
                        {
                            return None;
                        }
                        Some(ListItem::avatar(
                            login_avatar(theme),
                            label(&transfer.entry),
                            login_subtitle(&transfer.entry),
                            None,
                            Some(flow(app.incoming(instance.clone()))),
                        ))
                    })
                    .collect::<Vec<_>>();
                let (incoming, sent): (Vec<_>, Vec<_>) = requests
                    .into_iter()
                    .filter(|instance| {
                        let request = instance.pending();
                        request.validate().is_ok()
                            && (request.requester == app.name || request.recipient == app.name)
                            && !instance.confirmed().is_some_and(|request| {
                                request.response_saved
                                    || (request.recipient == app.name
                                        && request.status == RequestStatus::Responded)
                            })
                    })
                    .partition(|instance| instance.pending().recipient == app.name);
                if incoming.is_empty() && sent.is_empty() && received.is_empty() {
                    // CHK centers a root containing one non-list display item.
                    items.push(Display::instructions(
                        "No requests yet.\nYou can make one below.",
                    ));
                }
                if !received.is_empty() {
                    items.push(Display::list(Some("Received logins"), received, None));
                }
                if !incoming.is_empty() {
                    items.push(Display::list(
                        Some("Received"),
                        app.request_rows(&incoming, &profiles, theme),
                        None,
                    ));
                }
                if !sent.is_empty() {
                    items.push(Display::list(
                        Some("Sent"),
                        app.request_rows(&sent, &profiles, theme),
                        None,
                    ));
                }
                let app = app.clone();
                PageType::root(
                    "Requests",
                    vec![],
                    items,
                    None,
                    Some((
                        "New request".into(),
                        Box::new(move |ctx: &mut Context, theme: &Theme| {
                            app.new_request(ctx, theme)
                        }),
                    )),
                    None,
                )
            },
        )
    }

    fn new_request(&self, ctx: &mut Context, theme: &Theme) -> Flow {
        let selector = Selector::new("contact name", self.contact_items(ctx));
        let actions = vec![FormAction::Custom(
            "New contact".into(),
            None,
            Icons::Add,
            Action::flow(self.add_share_contact(theme, selector.clone())),
        )];
        let app = self.clone();
        Flow::from_form(Form::flow(
            theme,
            vec![
                FormItem::selection("New request", selector.with_actions(actions)),
                request_subject(),
            ],
            None,
            None,
            Box::new(move |ctx: &mut Context, values: &Vec<State>| {
                let [State::Selection(recipients), State::Text(subject)] = values.as_slice() else {
                    return FormComplete::None;
                };
                let Some(recipients) = app.selected_contacts(recipients) else {
                    return FormComplete::None;
                };
                app.confirm_request(ctx, recipients, subject)
            }),
        ))
    }

    fn request_rows(
        &self,
        requests: &[Instance<LoginRequest>],
        profiles: &[Instance<Profile>],
        theme: &Theme,
    ) -> Vec<ListItem> {
        let mut requests: Vec<_> = requests
            .iter()
            .filter(|instance| {
                let confirmed = instance.confirmed();
                let request = confirmed
                    .as_ref()
                    .map(|request| (**request).clone())
                    .unwrap_or_else(|| instance.pending().clone());
                let participant = request.requester == self.name || request.recipient == self.name;
                participant && request.validate().is_ok()
            })
            .cloned()
            .collect();
        requests.sort_by(|a, b| {
            b.pending()
                .created_at
                .cmp(&a.pending().created_at)
                .then_with(|| a.id().cmp(&b.id()))
        });
        requests
            .into_iter()
            .map(|instance| {
                let request = instance.pending();
                let peer = if request.requester == self.name {
                    request.recipient
                } else {
                    request.requester
                };
                let name = contact_label(profiles, peer);
                let confirmed = instance.confirmed();
                let status = if confirmed.as_ref().is_none_or(|value| **value != *request) {
                    pending_status(&request, confirmed.is_none())
                } else {
                    request_status(&request, self.name)
                };
                let subtitle = format!("{name} · {status}");
                ListItem::avatar(
                    request_avatar(theme),
                    &request_title(&request),
                    &subtitle,
                    None,
                    Some(flow(self.request_details(instance.clone()))),
                )
                .with_badge(
                    (request.recipient == self.name
                        && matches!(
                            request.status,
                            RequestStatus::Pending | RequestStatus::Accepted
                        ))
                        || (request.requester == self.name
                            && request.status == RequestStatus::Responded
                            && !request.response_saved),
                )
            })
            .collect()
    }

    pub(super) fn request_password(&self, recipient: Name, theme: &Theme) -> Flow {
        let app = self.clone();
        Flow::from_form(Form::flow(
            theme,
            vec![request_subject()],
            None,
            None,
            Box::new(move |ctx: &mut Context, values: &Vec<State>| {
                let Some(State::Text(subject)) = values.first() else {
                    return FormComplete::None;
                };
                app.confirm_request(ctx, vec![recipient], subject)
            }),
        ))
    }

    fn confirm_request(
        &self,
        ctx: &mut Context,
        recipients: Vec<Name>,
        subject: &str,
    ) -> FormComplete {
        let Ok(subject) = requests::validate_subject(subject) else {
            return FormComplete::None;
        };
        let recipients = recipients.into_iter().collect::<BTreeSet<_>>();
        if recipients.is_empty() || recipients.contains(&self.name) {
            return FormComplete::None;
        }
        let profiles = ctx
            .air
            .instances::<Profile>()
            .list()
            .into_values()
            .collect::<Vec<_>>();
        let mut items = recipients
            .iter()
            .map(|recipient| {
                Display::cta(
                    "To",
                    Some(&contact_label(&profiles, *recipient)),
                    &recipient.to_string(),
                    vec![],
                )
            })
            .collect::<Vec<_>>();
        items.push(Display::cta("App or website", Some(&subject), "", vec![]));
        let app = self.clone();
        FormComplete::Next(page(PageType::display(
            "Confirm request",
            items,
            None,
            Bumper::custom(
                "Send request",
                Action::custom(move |ctx: &mut Context, theme: &Theme| {
                    let result = recipients
                        .iter()
                        .map(|recipient| requests::send_request(ctx, *recipient, subject.clone()))
                        .collect::<Result<Vec<_>, _>>();
                    let next = match result {
                        Ok(requests) => {
                            app.changes.notify();
                            app.request_sent(requests, subject.clone())
                        }
                        Err(error) => page(message("Request password", &error)),
                    };
                    FormComplete::Next(next).run(ctx, theme);
                }),
            ),
            Offset::Start,
        )))
    }

    fn request_sent(&self, sent: Vec<Instance<LoginRequest>>, subject: String) -> Page {
        let ids = sent.iter().map(Instance::id).collect::<Vec<_>>();
        Page::watching(
            Watch::list::<LoginRequest>(),
            move |_: &mut Context, _: &Theme, requests: Vec<Instance<LoginRequest>>| {
                let confirmed = ids.iter().all(|id| {
                    requests
                        .iter()
                        .find(|instance| instance.id() == *id)
                        .is_some_and(|instance| instance.confirmed().is_some())
                });
                let items = if confirmed {
                    vec![
                        Display::image("illustrations/request-sent", (160.0, 160.0)),
                        Display::confirmation_message(&format!("Requested '{subject}'")),
                    ]
                } else {
                    vec![Display::instructions("Sending request…")]
                };
                PageType::display("", items, None, Bumper::Done, Offset::Center)
            },
        )
    }

    fn request_details(&self, request: Instance<LoginRequest>) -> Page {
        let app = self.clone();
        let signal = Signal::default();
        let saving = Rc::new(RefCell::new(false));
        let notice = Rc::new(RefCell::new(String::new()));
        let pending_update = Rc::new(RefCell::new(None::<LoginRequest>));
        Page::watching(
            (
                Watch::contract(request),
                Watch::contract(self.session.borrow().instance()),
                Watch::list::<Profile>(),
                signal.clone(),
            ),
            move |ctx: &mut Context, theme: &Theme, (mut instance, _, profiles, ())| {
                if *saving.borrow() {
                    match requests::receive_response(
                        ctx,
                        &mut app.session.borrow_mut(),
                        &mut instance,
                    ) {
                        Ok(true) => {
                            app.changes.notify();
                            return PageType::display(
                                "Logins saved",
                                vec![
                                    Display::image("illustrations/login-saved", (160.0, 160.0)),
                                    Display::confirmation_message("The shared logins are saved."),
                                ],
                                None,
                                Bumper::Done,
                                Offset::Center,
                            );
                        }
                        Ok(false) => *notice.borrow_mut() = "Saving logins…".into(),
                        Err(error) => {
                            *saving.borrow_mut() = false;
                            *notice.borrow_mut() = error;
                        }
                    }
                }
                let request = instance.pending();
                if request.validate().is_err()
                    || (request.requester != app.name && request.recipient != app.name)
                {
                    return message("Password request", "This password request is unavailable.");
                }
                let incoming = request.recipient == app.name;
                let peer = if incoming {
                    request.requester
                } else {
                    request.recipient
                };
                if !incoming && request.status != RequestStatus::Responded {
                    return PageType::display(
                        if request.status == RequestStatus::Denied {
                            "Request denied"
                        } else {
                            "Password request"
                        },
                        vec![
                            Display::image("illustrations/password-request", (96.0, 96.0)),
                            Display::cta("Requested", Some(&request.subject), "", vec![]),
                            Display::cta("To", Some(&contact_label(&profiles, peer)), "", vec![]),
                        ],
                        None,
                        Bumper::Done,
                        Offset::Start,
                    );
                }
                let mut items = vec![Display::cta(
                    "From",
                    Some(&contact_label(&profiles, peer)),
                    &peer.to_string(),
                    vec![],
                )];
                if request.status != RequestStatus::Responded {
                    items.push(Display::cta(
                        "App or website",
                        Some(&request.subject),
                        "",
                        vec![],
                    ));
                }
                let mut bumper = Bumper::Done;
                let confirmed = instance.confirmed();
                let updating = confirmed.as_ref().is_none_or(|value| **value != *request);
                if updating {
                    *pending_update.borrow_mut() = Some(request.clone());
                } else if let Some(expected) = pending_update.borrow_mut().take() {
                    if !update_confirmed(&expected, &request) {
                        *notice.borrow_mut() =
                            "This update wasn’t confirmed. Please try again.".into();
                    }
                }
                if updating {
                    items.push(Display::instructions(pending_status(
                        &request,
                        confirmed.is_none(),
                    )));
                } else if incoming && request.status == RequestStatus::Responded {
                    return shared_success_type();
                } else if incoming && request.status == RequestStatus::Denied {
                    return PageType::display(
                        "Request denied",
                        vec![Display::confirmation_message("The request was denied.")],
                        None,
                        Bumper::Done,
                        Offset::Center,
                    );
                } else if incoming
                    && matches!(
                        request.status,
                        RequestStatus::Pending | RequestStatus::Accepted
                    )
                {
                    let mut accepted = instance.clone();
                    let choose_app = app.clone();
                    let error_notice = notice.clone();
                    let update = signal.clone();
                    let accept = Action::custom(move |ctx: &mut Context, theme: &Theme| {
                        match requests::accept(ctx, &mut accepted) {
                            Ok(()) => {
                                choose_app.changes.notify();
                                FormComplete::Next(
                                    choose_app.select_response(accepted.clone(), theme),
                                )
                                .run(ctx, theme);
                            }
                            Err(error) => {
                                *error_notice.borrow_mut() = error;
                                update.notify();
                            }
                        }
                    });
                    let mut denied = instance.clone();
                    let deny_app = app.clone();
                    let error_notice = notice.clone();
                    let update = signal.clone();
                    bumper = Bumper::double(
                        "Accept",
                        accept,
                        "Deny",
                        Action::custom(move |ctx: &mut Context, _: &Theme| {
                            match requests::deny(ctx, &mut denied) {
                                Ok(()) => {
                                    deny_app.changes.notify();
                                    update.notify();
                                }
                                Err(error) => {
                                    *error_notice.borrow_mut() = error;
                                    update.notify();
                                }
                            }
                        }),
                    );
                } else if !incoming && request.status == RequestStatus::Responded {
                    let rows = request
                        .responses
                        .iter()
                        .map(|entry| {
                            ListItem::avatar(
                                login_avatar(theme),
                                label(entry),
                                login_subtitle(entry),
                                None,
                                Some(flow(page(PageType::display(
                                    "Shared login",
                                    entry_items(entry, false),
                                    None,
                                    Bumper::Done,
                                    Offset::Start,
                                )))),
                            )
                        })
                        .collect();
                    items.push(Display::list(Some("Received logins"), rows, None));
                    if !request.response_saved && !*saving.borrow() {
                        let saving = saving.clone();
                        let signal = signal.clone();
                        bumper = Bumper::custom(
                            "Save all",
                            Action::custom(move |_: &mut Context, _: &Theme| {
                                *saving.borrow_mut() = true;
                                signal.notify();
                            }),
                        );
                    } else if *saving.borrow() {
                        bumper = Bumper::None;
                    } else {
                        items.push(Display::instructions("These logins have been saved."));
                    }
                } else {
                    items.push(Display::confirmation_message(request_status(
                        &request, app.name,
                    )));
                }
                if !notice.borrow().is_empty() {
                    items.push(Display::instructions(&notice.borrow()));
                }
                PageType::display("Password request", items, None, bumper, Offset::Start)
            },
        )
    }

    fn select_response(&self, mut instance: Instance<LoginRequest>, theme: &Theme) -> Page {
        let request = instance.pending();
        if request.recipient != self.name
            || request.validate().is_err()
            || request.status != RequestStatus::Accepted
        {
            return page(message(
                "Choose passwords",
                "This request is no longer waiting for logins.",
            ));
        }
        drop(request);
        let selector = match self.login_selector(theme) {
            Ok(selector) => selector,
            Err(error) => return page(message("Choose passwords", &error)),
        };
        let app = self.clone();
        // Use a standard single-page form in the existing flow. Returning its page
        // keeps Done and Back on the same navigation stack as the request.
        Flow::from_form(Form::flow(
            theme,
            vec![FormItem::selection("Choose passwords", selector)],
            None,
            None,
            Box::new(move |ctx: &mut Context, values: &Vec<State>| {
                let Some(State::Selection(ids)) = values.first() else {
                    return FormComplete::None;
                };
                // Reload every selected login before sending anything. The request
                // model also rechecks the recipient, status, and response bounds.
                let result = app
                    .selected_entries(ids)
                    .and_then(|entries| requests::respond(ctx, &mut instance, entries));
                match result {
                    Ok(()) => {
                        app.changes.notify();
                        FormComplete::Next(app.request_details(instance.clone()))
                    }
                    Err(error) => FormComplete::Next(page(message("Choose passwords", &error))),
                }
            }),
        ))
        .0
        .remove(0)
    }
}

fn request_subject() -> FormItem {
    FormItem::text(
        "Request password",
        "App or website",
        None,
        |_: &mut Context, value: String| {
            if value.trim().is_empty() {
                FormValidState::Invalid
            } else {
                FormValidState::from(requests::validate_subject(&value).map(|_| String::new()))
            }
        },
    )
}

fn contact_label(profiles: &[Instance<Profile>], name: Name) -> String {
    profiles
        .iter()
        .find_map(|profile| {
            let profile = profile.pending();
            (profile.name == Some(name)).then(|| profile.username.clone())
        })
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| short_name(name))
}

fn request_avatar(theme: &Theme) -> AvatarContent {
    theme
        .brand()
        .images
        .get("illustrations/password-request")
        .cloned()
        .map(AvatarContent::image)
        .unwrap_or_else(|| login_avatar(theme))
}

fn request_title(request: &LoginRequest) -> String {
    if request.status != RequestStatus::Responded {
        return request.subject.clone();
    }
    let mut seen = BTreeSet::new();
    let names = request
        .responses
        .iter()
        .map(label)
        .filter(|name| seen.insert(*name))
        .collect::<Vec<_>>();
    let mut title = names.iter().take(2).copied().collect::<Vec<_>>().join(", ");
    if names.len() > 2 {
        title.push_str(&format!(" + {} more", names.len() - 2));
    }
    title
}

fn update_confirmed(expected: &LoginRequest, actual: &LoginRequest) -> bool {
    match expected.status {
        RequestStatus::Pending => true,
        RequestStatus::Accepted => actual.status != RequestStatus::Pending,
        RequestStatus::Denied => actual.status == RequestStatus::Denied,
        RequestStatus::Responded => {
            actual.status == RequestStatus::Responded
                && actual.responses == expected.responses
                && (!expected.response_saved || actual.response_saved)
        }
    }
}

fn pending_status(request: &LoginRequest, creating: bool) -> &'static str {
    if creating {
        "Sending request…"
    } else if request.status == RequestStatus::Responded {
        if request.response_saved {
            "Saving logins…"
        } else {
            "Sending logins…"
        }
    } else {
        "Updating request…"
    }
}

fn request_status(request: &LoginRequest, me: Name) -> &'static str {
    match request.status {
        RequestStatus::Pending if request.recipient == me => "Waiting for you",
        RequestStatus::Pending => "Request sent",
        RequestStatus::Accepted if request.recipient == me => "Choose passwords",
        RequestStatus::Accepted => "Accepted",
        RequestStatus::Denied => "Request denied",
        RequestStatus::Responded if request.response_saved => "Logins saved",
        RequestStatus::Responded if request.requester == me => "Logins ready to save",
        RequestStatus::Responded => "Logins sent",
    }
}
