//! All pages, inputs, navigation, and controls are composed through CHK.
use crate::{
    contacts,
    passphrase::{PassphraseOptions, generate_passphrase},
    runtime::{ClipboardUi, copy_action},
    sharing::{self, LoginTransfer, TransferStatus},
    site_logos::SiteLogos,
    storage::{Entry, PasswordStore},
};
use chk::profiles::{Profile, ProfileAction};
use chk::{
    Action, ActionItem, App, Assets, AvatarContent, AvatarIconStyle, Bumper, ChkTheme, Color,
    Context, Display, Flow, Form, FormAction, FormComplete, FormItem, FormValidState, Icons, Input,
    ListItem, Offset, Page, PageType, Root, RootInfo, State, Theme, Watch, Watcher,
};
use prism::air::{Instance, Name};
use std::{
    cell::RefCell,
    future::poll_fn,
    rc::Rc,
    task::{Poll, Waker},
};
use uuid::Uuid;

type Session = Rc<RefCell<PasswordStore>>;

#[derive(Clone)]
pub struct PasswordsApp {
    session: Session,
    logos: SiteLogos,
    name: Name,
    changes: Signal,
}
impl PasswordsApp {
    pub fn new(ctx: &mut Context) -> Self {
        sharing::open(&ctx.air);
        Self {
            session: Rc::new(RefCell::new(PasswordStore::new(ctx))),
            logos: SiteLogos::default(),
            name: ctx.air.me(),
            changes: Signal::default(),
        }
    }
    pub fn build_ui(self, ctx: &mut Context, assets: Assets) -> ClipboardUi {
        ClipboardUi::new(Box::new(self.build(ctx, assets)))
    }
    fn entry(&self, id: &str) -> Result<Entry, String> {
        self.session
            .borrow_mut()
            .entries()
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| "This login no longer exists.".into())
    }
    fn home(&self) -> Page {
        let app = self.clone();
        Page::watching(
            (
                Watch::contract(self.session.borrow().instance()),
                Watch::list::<LoginTransfer>(),
                LogoWatch(self.logos.clone()),
                self.changes.clone(),
            ),
            move |ctx: &mut Context, theme: &Theme, (_, transfers, _, ())| {
                let mut items = vec![];
                if let Err(error) =
                    sharing::reconcile_received(ctx, &mut app.session.borrow_mut(), &transfers)
                {
                    items.push(Display::text(&error));
                }
                let entries = match app.session.borrow_mut().entries() {
                    Ok(entries) => entries,
                    Err(error) => {
                        items.push(Display::text(&error.to_string()));
                        vec![]
                    }
                };
                let rows = entries
                    .iter()
                    .map(|entry| {
                        ListItem::avatar(
                            app.logos
                                .get(label(entry), &entry.url)
                                .unwrap_or_else(|| login_avatar(theme)),
                            label(entry),
                            login_subtitle(entry),
                            None,
                            Some(flow(app.details(entry.id.clone()))),
                        )
                    })
                    .collect();
                if entries.is_empty() {
                    items.push(Display::image("illustrations/peanut-vault", (96.0, 96.0)));
                }
                items.push(Display::list(
                    None,
                    rows,
                    entries
                        .is_empty()
                        .then_some("A fresh start. Add your first password."),
                ));
                let incoming = transfers
                    .into_iter()
                    .filter_map(|instance| {
                        let transfer = instance.confirmed()?;
                        if transfer.recipient != app.name
                            || transfer.status != TransferStatus::Pending
                            || transfer.validate().is_err()
                        {
                            return None;
                        }
                        let row = ListItem::avatar(
                            login_avatar(theme),
                            label(&transfer.entry),
                            "Ready to review",
                            None,
                            Some(flow(app.incoming(instance.clone()))),
                        );
                        Some(row)
                    })
                    .collect::<Vec<_>>();
                if !incoming.is_empty() {
                    items.push(Display::list(Some("Received logins"), incoming, None));
                }
                let name = app.name;
                let editor = app.clone();
                PageType::root(
                    "Passwords",
                    vec![],
                    items,
                    Some((
                        Icons::QrCode,
                        Box::new(move |_: &mut Context, _: &Theme| flow(name_page(name))),
                    )),
                    Some((
                        "Add password".into(),
                        Box::new(move |_: &mut Context, _: &Theme| editor.editor(Entry::new())),
                    )),
                    None,
                )
            },
        )
    }
    fn details(&self, id: String) -> Page {
        let app = self.clone();
        Page::watching(
            (
                Watch::contract(self.session.borrow().instance()),
                self.changes.clone(),
            ),
            move |_: &mut Context, _: &Theme, _| {
                let entry = match app.entry(&id) {
                    Ok(entry) => entry,
                    Err(error) => return message("Login details", &error),
                };
                let mut items = entry_items(&entry, true);
                if !entry.url.is_empty() {
                    let url = entry.url.clone();
                    items.insert(
                        3,
                        Display::cta(
                            "Link",
                            Some(&entry.url),
                            "",
                            vec![
                                ActionItem::new(
                                    Action::custom(move |ctx: &mut Context, theme: &Theme| {
                                        if let Err(error) = crate::links::open(&url) {
                                            FormComplete::Next(page(message("Open link", &error)))
                                                .run(ctx, theme);
                                        }
                                    }),
                                    "Open link",
                                    Icons::Link,
                                ),
                                copy_action("Copy link", &entry.url),
                            ],
                        ),
                    );
                }
                let edit_app = app.clone();
                let edit_id = id.clone();
                let share_app = app.clone();
                let share_id = id.clone();
                PageType::display(
                    "Login details",
                    items,
                    Some((
                        Icons::Edit,
                        Box::new(move |_: &mut Context, _: &Theme| {
                            match edit_app.entry(&edit_id) {
                                Ok(entry) => edit_app.editor(entry),
                                Err(error) => flow(page(message("Edit login", &error))),
                            }
                        }),
                    )),
                    Bumper::custom(
                        "Share",
                        Action::custom(move |ctx: &mut Context, theme: &Theme| {
                            let mut next = share_app.share_login(ctx, share_id.clone(), theme);
                            (next.build(ctx, theme))(ctx, theme);
                        }),
                    ),
                    Offset::Start,
                )
            },
        )
    }
    fn editor(&self, entry: Entry) -> Flow {
        let state = Editor::new(entry);
        self.editor_with(state)
    }
    fn editor_with(&self, editor: Editor) -> Flow {
        let app = self.clone();
        flow(Page::watching(
            editor.signal.clone(),
            move |ctx: &mut Context, theme: &Theme, ()| {
                let snapshot = editor.draft.borrow().clone();
                let mut inputs = vec![];
                let mut validations = vec![];
                for field in Field::ALL {
                    let id = editor.ids[field as usize];
                    let baseline = field.value(&editor.initial).to_owned();
                    let current = field.value(&snapshot).to_owned();
                    let generator = editor.clone();
                    let actions = (field == Field::Password).then(|| {
                        vec![
                            (
                                "Paste".into(),
                                Some("Pasted".into()),
                                Icons::Paste,
                                Action::Paste { id },
                            ),
                            (
                                "Generate".into(),
                                None,
                                Icons::Infinite,
                                Action::custom(move |ctx: &mut Context, theme: &Theme| {
                                    match generate_passphrase(*generator.options.borrow()) {
                                        Ok(value) => {
                                            (Action::set_text(id, value).get())(ctx, theme)
                                        }
                                        Err(error) => {
                                            *generator.notice.borrow_mut() = error.to_string();
                                            generator.signal.notify();
                                        }
                                    }
                                }),
                            ),
                        ]
                    });
                    inputs.push(Input::text(
                        id,
                        field.label(),
                        true,
                        Some(baseline.clone()),
                        actions,
                    ));
                    let draft = editor.draft.clone();
                    let touched = editor.touched.clone();
                    validations.push(
                        FormItem::text_with_preset(
                            field.label(),
                            &baseline,
                            None,
                            move |_: &mut Context, value: String| {
                                field.set(&mut draft.borrow_mut(), value.clone());
                                if field == Field::Password {
                                    *touched.borrow_mut() |= !value.is_empty();
                                    if value.is_empty() {
                                        return if *touched.borrow() {
                                            FormValidState::InvalidWithData(
                                                "Enter a password.".into(),
                                            )
                                        } else {
                                            FormValidState::Invalid
                                        };
                                    }
                                }
                                FormValidState::from(
                                    text_field_result(field, &value).map(|_| String::new()),
                                )
                            },
                        )
                        .validation(),
                    );
                    // Keep the saved baseline for CHK's changed-value Save gating while
                    // restoring the current draft after an option refresh.
                    if current != baseline {
                        (Action::set_text(id, current).get())(ctx, theme);
                    }
                }
                let mut display = vec![editor.options_display()];
                if !editor.notice.borrow().is_empty() {
                    display.push(Display::text(&editor.notice.borrow()));
                }
                if !editor.initial.id.is_empty() {
                    let delete_app = app.clone();
                    let id = editor.initial.id.clone();
                    display.push(Display::actions(
                        vec![ActionItem::new(
                            Action::flow(flow(page(PageType::display(
                                "Delete password?",
                                vec![],
                                None,
                                Bumper::double(
                                    "Delete",
                                    Action::custom(move |ctx: &mut Context, theme: &Theme| {
                                        let result = delete_app.session.borrow_mut().delete(&id);
                                        match result {
                                            Ok(()) => {
                                                delete_app.changes.notify();
                                                return_to_list(ctx, theme);
                                            }
                                            Err(error) => FormComplete::Next(page(message(
                                                "Delete password",
                                                &error.to_string(),
                                            )))
                                            .run(ctx, theme),
                                        }
                                    }),
                                    "Cancel",
                                    Action::None,
                                ),
                                Offset::Center,
                            )))),
                            "Delete",
                            Icons::Delete,
                        )],
                        false,
                    ));
                }
                let saver = app.clone();
                let save_editor = editor.clone();
                let save_theme = theme.clone();
                PageType::edit(
                    if editor.initial.id.is_empty() {
                        "Add password"
                    } else {
                        "Edit login"
                    },
                    inputs,
                    display,
                    validations,
                    Box::new(move |ctx: &mut Context, values: &Vec<State>| {
                        let mut entry = save_editor.initial.clone();
                        for (field, value) in Field::ALL.into_iter().zip(values) {
                            if let State::Text(value) = value {
                                field.set(&mut entry, value.clone());
                            }
                        }
                        if !valid_entry(&entry) {
                            return FormComplete::None;
                        }
                        entry.title = entry_title(&entry);
                        let is_new = entry.id.is_empty();
                        let result = saver.session.borrow_mut().save(entry);
                        match result {
                            Ok(_) => {
                                saver.changes.notify();
                                if is_new {
                                    FormComplete::Next(success()).run(ctx, &save_theme);
                                } else {
                                    done(ctx, &save_theme);
                                }
                            }
                            Err(error) => {
                                *save_editor.notice.borrow_mut() = error.to_string();
                                save_editor.signal.notify();
                            }
                        }
                        FormComplete::None
                    }),
                )
            },
        ))
    }
    fn contacts(&self, login: Option<String>) -> Page {
        let app = self.clone();
        Page::watching(
            (
                Watch::list::<LoginTransfer>(),
                Watch::list::<Profile>(),
                self.changes.clone(),
            ),
            move |_: &mut Context, _: &Theme, (transfers, profiles, ())| {
                let contacts = contacts::from_sources(app.name, &transfers, &profiles);
                let rows = contacts
                    .iter()
                    .map(|contact| {
                        let next = match &login {
                            Some(id) => app.confirm_recipient(id.clone(), contact.name),
                            None => app.contact(contact.name),
                        };
                        let profile = profiles
                            .iter()
                            .find(|profile| profile.pending().name == Some(contact.name));
                        let (avatar, title, subtitle) = match profile {
                            Some(profile) => {
                                let profile = profile.pending();
                                (
                                    profile.avatar.clone(),
                                    profile.username.clone(),
                                    format!(
                                        "{} · {}",
                                        short_name(contact.name),
                                        share_count(contact.share_count)
                                    ),
                                )
                            }
                            None => (
                                AvatarContent::icon(Icons::Profile, AvatarIconStyle::Brand),
                                short_name(contact.name),
                                share_count(contact.share_count),
                            ),
                        };
                        ListItem::avatar(avatar, &title, &subtitle, None, Some(flow(next)))
                    })
                    .collect();
                let items = vec![Display::list(
                    None,
                    rows,
                    Some("Add a contact or share a login to get started."),
                )];
                match &login {
                    None => {
                        let app = app.clone();
                        PageType::root(
                            "Contacts",
                            vec![],
                            items,
                            None,
                            Some((
                                "Add contact".into(),
                                Box::new(move |_: &mut Context, theme: &Theme| {
                                    app.add_contact(theme)
                                }),
                            )),
                            None,
                        )
                    }
                    Some(id) => {
                        let app = app.clone();
                        let id = id.clone();
                        PageType::display(
                            "Share login",
                            items,
                            None,
                            Bumper::custom(
                                "Someone new",
                                Action::custom(move |ctx: &mut Context, theme: &Theme| {
                                    let mut next = app.recipient(id.clone(), theme);
                                    (next.build(ctx, theme))(ctx, theme);
                                }),
                            ),
                            Offset::Start,
                        )
                    }
                }
            },
        )
    }
    fn add_contact(&self, theme: &Theme) -> Flow {
        let app = self.clone();
        let me = self.name;
        Flow::from_form(Form::flow(
            theme,
            vec![FormItem::text(
                "Add contact",
                "Peanut Butter Name",
                Some(vec![
                    FormAction::Paste,
                    FormAction::scan_qr_code("Scan their Peanut Butter Name"),
                ]),
                move |_: &mut Context, value: String| {
                    if value.trim().is_empty() {
                        FormValidState::Invalid
                    } else {
                        FormValidState::from(sharing::recipient(&value, me).map(|_| String::new()))
                    }
                },
            )],
            None,
            None,
            Box::new(move |ctx: &mut Context, values: &Vec<State>| {
                let Some(State::Text(value)) = values.first() else {
                    return FormComplete::None;
                };
                let Ok(name) = sharing::recipient(value, me) else {
                    return FormComplete::None;
                };
                // Orange's existing contact contract; adding a contact never sends a login.
                Profile::from_name(ctx, name);
                app.changes.notify();
                FormComplete::Next(app.contact(name))
            }),
        ))
    }
    fn share_login(&self, ctx: &mut Context, id: String, theme: &Theme) -> Flow {
        let transfers: Vec<_> = sharing::open(&ctx.air).list().into_values().collect();
        let profiles: Vec<_> = ctx
            .air
            .instances::<Profile>()
            .list()
            .into_values()
            .collect();
        if contacts::from_sources(self.name, &transfers, &profiles).is_empty() {
            self.recipient(id, theme)
        } else {
            flow(self.contacts(Some(id)))
        }
    }
    fn contact(&self, name: Name) -> Page {
        let app = self.clone();
        Page::watching(
            (
                Watch::list::<LoginTransfer>(),
                Watch::list::<Profile>(),
                self.changes.clone(),
            ),
            move |_: &mut Context, _: &Theme, (transfers, profiles, ())| {
                let mut history = transfers
                    .into_iter()
                    .filter(|instance| {
                        let transfer = instance.pending();
                        transfer.sender == app.name
                            && transfer.recipient == name
                            && transfer.validate().is_ok()
                    })
                    .collect::<Vec<_>>();
                history.sort_by(|a, b| {
                    b.pending()
                        .created_at
                        .cmp(&a.pending().created_at)
                        .then_with(|| a.id().cmp(&b.id()))
                });
                let rows = history
                    .iter()
                    .map(|instance| {
                        let transfer = instance.pending();
                        let status = transfer_status(instance);
                        let username = login_username(&transfer.entry);
                        let subtitle = if username.is_empty() {
                            status.to_owned()
                        } else {
                            format!("{username} · {status}")
                        };
                        ListItem::plain(label(&transfer.entry), &subtitle, None, None)
                    })
                    .collect();
                let profile = profiles
                    .into_iter()
                    .find(|profile| profile.pending().name == Some(name));
                let mut items = vec![];
                let header = profile.map(|profile| {
                    items.push(Display::cta(
                        "Contact name",
                        Some(&profile.pending().username),
                        "",
                        vec![],
                    ));
                    let app = app.clone();
                    (
                        Icons::Edit,
                        Box::new(move |_: &mut Context, theme: &Theme| {
                            app.edit_contact(profile.clone(), theme)
                        }) as Box<dyn chk::FlowBuilder>,
                    )
                });
                items.extend([
                    Display::cta(
                        "Peanut Butter Name",
                        Some(&name.to_string()),
                        "",
                        vec![copy_action("Copy", &name.to_string())],
                    ),
                    Display::list(Some("Shared logins"), rows, Some("No shared logins yet.")),
                ]);
                let app = app.clone();
                PageType::display(
                    "Contact",
                    items,
                    header,
                    Bumper::custom(
                        "Share a login",
                        Action::custom(move |ctx: &mut Context, theme: &Theme| {
                            let mut next = flow(app.choose_login(name));
                            (next.build(ctx, theme))(ctx, theme);
                        }),
                    ),
                    Offset::Start,
                )
            },
        )
    }
    fn edit_contact(&self, mut profile: Instance<Profile>, theme: &Theme) -> Flow {
        let username = profile.pending().username.clone();
        let app = self.clone();
        let theme = theme.clone();
        Flow::from_form(Form::page(
            "Edit contact",
            vec![FormItem::text_with_preset(
                "Contact name",
                &username,
                None,
                |_: &mut Context, value: String| {
                    if value.trim().is_empty() {
                        FormValidState::InvalidWithData("Enter a contact name.".into())
                    } else if value.len() > 80 {
                        FormValidState::InvalidWithData("This contact name is too long.".into())
                    } else {
                        FormValidState::Valid
                    }
                },
            )],
            vec![],
            Box::new(move |ctx: &mut Context, values: &Vec<State>| {
                if let Some(State::Text(value)) = values.first()
                    && !value.trim().is_empty()
                    && value.len() <= 80
                {
                    profile.send(ProfileAction::ChangeUsername(value.trim().to_owned()));
                    app.changes.notify();
                    done(ctx, &theme);
                }
                FormComplete::None
            }),
        ))
    }
    fn choose_login(&self, recipient: Name) -> Page {
        let app = self.clone();
        Page::watching(
            (
                Watch::contract(self.session.borrow().instance()),
                self.changes.clone(),
                LogoWatch(self.logos.clone()),
            ),
            move |_: &mut Context, theme: &Theme, _| {
                let entries = match app.session.borrow_mut().entries() {
                    Ok(entries) => entries,
                    Err(error) => return message("Choose login", &error.to_string()),
                };
                let rows = entries
                    .iter()
                    .map(|entry| {
                        ListItem::avatar(
                            app.logos
                                .get(label(entry), &entry.url)
                                .unwrap_or_else(|| login_avatar(theme)),
                            label(entry),
                            login_subtitle(entry),
                            None,
                            Some(flow(app.confirm_recipient(entry.id.clone(), recipient))),
                        )
                    })
                    .collect();
                PageType::display(
                    "Choose login",
                    vec![Display::list(
                        None,
                        rows,
                        Some("Add a password before sharing a login."),
                    )],
                    None,
                    Bumper::None,
                    Offset::Start,
                )
            },
        )
    }
    fn recipient(&self, id: String, theme: &Theme) -> Flow {
        let me = self.name;
        let app = self.clone();
        Flow::from_form(Form::flow(
            theme,
            vec![FormItem::text_with_preset(
                "Peanut Butter Name",
                "",
                Some(vec![
                    FormAction::Paste,
                    FormAction::scan_qr_code("Scan their Peanut Butter Name"),
                ]),
                move |_: &mut Context, value: String| {
                    if value.trim().is_empty() {
                        FormValidState::Invalid
                    } else {
                        FormValidState::from(sharing::recipient(&value, me).map(|_| String::new()))
                    }
                },
            )],
            None,
            None,
            Box::new(move |_: &mut Context, values: &Vec<State>| {
                let Some(State::Text(value)) = values.first() else {
                    return FormComplete::None;
                };
                match sharing::recipient(value, me) {
                    Ok(recipient) => {
                        FormComplete::Next(app.confirm_recipient(id.clone(), recipient))
                    }
                    Err(_) => FormComplete::None,
                }
            }),
        ))
    }
    fn confirm_recipient(&self, id: String, recipient: Name) -> Page {
        let app = self.clone();
        let entry = match self.entry(&id) {
            Ok(entry) => entry,
            Err(error) => return page(message("Share login", &error)),
        };
        page(PageType::display(
            "Confirm recipient",
            vec![
                Display::cta("Login", Some(label(&entry)), login_username(&entry), vec![]),
                Display::cta("Send to", Some(&recipient.to_string()), "", vec![]),
                Display::instructions(
                    "Sends a copy of the app or website name, username, password, link, and notes.",
                ),
            ],
            None,
            Bumper::custom(
                "Send",
                Action::custom(move |ctx: &mut Context, theme: &Theme| {
                    let result = app
                        .entry(&id)
                        .and_then(|entry| sharing::send(ctx, entry, recipient));
                    let next = match result {
                        Ok(instance) => {
                            app.changes.notify();
                            outgoing(instance)
                        }
                        Err(error) => page(message("Share login", &error)),
                    };
                    FormComplete::Next(next).run(ctx, theme);
                }),
            ),
            Offset::Start,
        ))
    }
    fn incoming(&self, transfer: Instance<LoginTransfer>) -> Page {
        let app = self.clone();
        let signal = Signal::default();
        let saving = Rc::new(RefCell::new(false));
        let changes = signal.clone();
        Page::watching(
            (
                Watch::contract(transfer),
                Watch::contract(self.session.borrow().instance()),
                signal,
            ),
            move |ctx: &mut Context, _: &Theme, (mut instance, _, ())| {
                let mut notice = None;
                if *saving.borrow() {
                    match sharing::receive(ctx, &mut app.session.borrow_mut(), &mut instance) {
                        Ok(true) => {
                            app.changes.notify();
                            return success_type();
                        }
                        Ok(false) => notice = Some("Saving login…".into()),
                        Err(error) => {
                            *saving.borrow_mut() = false;
                            notice = Some(error);
                        }
                    }
                }
                let transfer = instance.pending();
                if transfer.recipient != app.name
                    || transfer.validate().is_err()
                    || instance.confirmed().is_none()
                {
                    return message("Received login", "This shared login is unavailable.");
                }
                let mut items = vec![Display::cta(
                    "From",
                    Some(&transfer.sender.to_string()),
                    "",
                    vec![],
                )];
                items.extend(entry_items(&transfer.entry, false));
                if let Some(notice) = notice {
                    items.push(Display::text(&notice));
                }
                let bumper = if transfer.status != TransferStatus::Pending {
                    Bumper::Done
                } else if *saving.borrow() {
                    Bumper::None
                } else {
                    let saving = saving.clone();
                    let signal = changes.clone();
                    let mut instance = instance.clone();
                    let app = app.clone();
                    Bumper::double(
                        "Save",
                        Action::custom(move |_: &mut Context, _: &Theme| {
                            *saving.borrow_mut() = true;
                            signal.notify();
                        }),
                        "Decline",
                        Action::custom(move |ctx: &mut Context, theme: &Theme| {
                            match sharing::decline(ctx, &mut instance) {
                                Ok(()) => {
                                    app.changes.notify();
                                    done(ctx, theme);
                                }
                                Err(error) => {
                                    FormComplete::Next(page(message("Received login", &error)))
                                        .run(ctx, theme)
                                }
                            }
                        }),
                    )
                };
                PageType::display("Received login", items, None, bumper, Offset::Start)
            },
        )
    }
}
impl App for PasswordsApp {
    fn theme(&self) -> ChkTheme {
        ChkTheme::Dark(Color::from_hex("#E9BC7B", 255))
    }
    fn roots(&self, ctx: &mut Context, theme: &Theme) -> Vec<RootInfo> {
        vec![
            RootInfo::icon(
                ctx,
                theme,
                Icons::Credential,
                "Passwords",
                Root::custom(self.home()),
            ),
            RootInfo::icon(
                ctx,
                theme,
                Icons::Group,
                "Contacts",
                Root::custom(self.contacts(None)),
            ),
        ]
    }
}
fn page(value: PageType) -> Page {
    Page::Static(Box::new(value))
}
fn flow(page: Page) -> Flow {
    Flow::new(vec![page])
}
fn label(entry: &Entry) -> &str {
    if !entry.title.trim().is_empty() {
        &entry.title
    } else if !entry.username.is_empty() {
        &entry.username
    } else if !entry.url.is_empty() {
        &entry.url
    } else {
        "Password"
    }
}
fn login_username(entry: &Entry) -> &str {
    if entry.username == label(entry) {
        ""
    } else {
        &entry.username
    }
}
fn login_subtitle(entry: &Entry) -> &str {
    if !entry.username.is_empty() && entry.username != label(entry) {
        &entry.username
    } else {
        &entry.url
    }
}
fn message(title: &str, message: &str) -> PageType {
    PageType::display(
        title,
        vec![Display::text(message)],
        None,
        Bumper::Done,
        Offset::Center,
    )
}
fn success_type() -> PageType {
    PageType::display(
        "Login created",
        vec![
            Display::image("illustrations/login-saved", (180.0, 180.0)),
            Display::confirmation_message("Your login is saved. Smooth work."),
        ],
        None,
        Bumper::Done,
        Offset::Center,
    )
}
fn success() -> Page {
    page(success_type())
}
fn done(ctx: &mut Context, theme: &Theme) {
    FormComplete::Return(Box::new(|_: &mut Context, _: &Theme| {})).run(ctx, theme);
}
// Confirmation is nested inside Edit, which may itself be opened from Details.
// Each CHK return closes one flow; returning from an already open root is harmless.
fn return_to_list(ctx: &mut Context, theme: &Theme) {
    for _ in 0..3 {
        done(ctx, theme);
    }
}
fn entry_items(entry: &Entry, copy: bool) -> Vec<Display> {
    let mut items = vec![
        Display::cta("App or website", Some(label(entry)), "", vec![]),
        Display::cta(
            "Username",
            Some(&entry.username),
            "",
            if copy {
                vec![copy_action("Copy", &entry.username)]
            } else {
                vec![]
            },
        ),
        Display::cta(
            "Password",
            Some(&entry.password),
            "",
            if copy {
                vec![copy_action("Copy", &entry.password)]
            } else {
                vec![]
            },
        ),
    ];
    if !copy && !entry.url.is_empty() {
        items.push(Display::cta("Link", Some(&entry.url), "", vec![]));
    }
    if !entry.notes.is_empty() {
        items.push(Display::cta("Notes", Some(&entry.notes), "", vec![]));
    }
    items
}
fn name_page(name: Name) -> Page {
    let name = name.to_string();
    page(PageType::display(
        "Share name",
        vec![Display::cta(
            "Peanut Butter Name",
            Some(&name),
            "",
            vec![
                copy_action("Copy", &name),
                ActionItem::new(
                    Action::flow(flow(page(PageType::display_qr_code(
                        "Peanut Butter Name",
                        &name,
                        "",
                        true,
                    )))),
                    "Show QR code",
                    Icons::QrCode,
                ),
            ],
        )],
        None,
        Bumper::None,
        Offset::Start,
    ))
}
fn short_name(name: Name) -> String {
    let name = name.to_string();
    if name.len() > 24 && name.is_ascii() {
        format!("{}…{}", &name[..10], &name[name.len() - 8..])
    } else {
        name
    }
}
fn share_count(count: usize) -> String {
    match count {
        0 => "No logins shared".into(),
        1 => "Shared once".into(),
        _ => format!("Shared {count} times"),
    }
}
fn transfer_status(instance: &Instance<LoginTransfer>) -> &'static str {
    match instance.confirmed() {
        None => "Queued for sync",
        Some(confirmed) => match confirmed.status {
            TransferStatus::Pending => "Waiting for recipient",
            TransferStatus::Accepted => "Accepted by recipient",
            TransferStatus::Declined => "Declined by recipient",
        },
    }
}
fn outgoing(instance: Instance<LoginTransfer>) -> Page {
    Page::contract_updates(
        instance,
        |_: &mut Context, _: &Theme, instance: Instance<LoginTransfer>| {
            let status = transfer_status(&instance);
            let transfer = instance.pending();
            PageType::display(
                "Sharing",
                vec![
                    Display::confirmation_message(status),
                    Display::cta(
                        "Login",
                        Some(label(&transfer.entry)),
                        login_username(&transfer.entry),
                        vec![],
                    ),
                    Display::cta(
                        "Recipient",
                        Some(&transfer.recipient.to_string()),
                        "",
                        vec![],
                    ),
                ],
                None,
                Bumper::Done,
                Offset::Start,
            )
        },
    )
}
fn login_avatar(theme: &Theme) -> AvatarContent {
    theme
        .brand()
        .images
        .get("login-icons/peanut")
        .cloned()
        .map(AvatarContent::image)
        .unwrap_or_else(|| AvatarContent::icon(Icons::Credential, AvatarIconStyle::Brand))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Username,
    Password,
    Url,
    Notes,
}
impl Field {
    const ALL: [Self; 5] = [
        Self::Title,
        Self::Username,
        Self::Password,
        Self::Url,
        Self::Notes,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Title => "App or website",
            Self::Username => "Username",
            Self::Password => "Password",
            Self::Url => "Link (optional)",
            Self::Notes => "Notes (optional)",
        }
    }
    fn value(self, entry: &Entry) -> &str {
        match self {
            Self::Title => &entry.title,
            Self::Username => &entry.username,
            Self::Password => &entry.password,
            Self::Url => &entry.url,
            Self::Notes => &entry.notes,
        }
    }
    fn set(self, entry: &mut Entry, value: String) {
        *match self {
            Self::Title => &mut entry.title,
            Self::Username => &mut entry.username,
            Self::Password => &mut entry.password,
            Self::Url => &mut entry.url,
            Self::Notes => &mut entry.notes,
        } = value;
    }
}
#[derive(Clone)]
struct Editor {
    initial: Entry,
    draft: Rc<RefCell<Entry>>,
    ids: [Uuid; 5],
    options: Rc<RefCell<PassphraseOptions>>,
    touched: Rc<RefCell<bool>>,
    notice: Rc<RefCell<String>>,
    signal: Signal,
}
impl Editor {
    fn new(initial: Entry) -> Self {
        Self {
            draft: Rc::new(RefCell::new(initial.clone())),
            initial,
            ids: std::array::from_fn(|_| Uuid::new_v4()),
            options: Rc::new(RefCell::new(PassphraseOptions::default())),
            touched: Rc::default(),
            notice: Rc::default(),
            signal: Signal::default(),
        }
    }
    fn options_display(&self) -> Display {
        let options = *self.options.borrow();
        let actions = [
            ("Capital letter", options.capital),
            ("Special character", options.special),
            ("Number", options.number),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (label, selected))| {
            let options = self.options.clone();
            let signal = self.signal.clone();
            ActionItem::new(
                Action::custom(move |_: &mut Context, _: &Theme| {
                    let mut options = options.borrow_mut();
                    let value = match index {
                        0 => &mut options.capital,
                        1 => &mut options.special,
                        _ => &mut options.number,
                    };
                    *value = !*value;
                    signal.notify();
                }),
                label,
                if selected {
                    Icons::Check
                } else {
                    Icons::Unchecked
                },
            )
        })
        .collect();
        Display::actions(actions, false)
    }
}
fn text_field_result(field: Field, value: &str) -> Result<(), String> {
    let limit = match field {
        Field::Title => 256,
        Field::Username | Field::Password => 4096,
        Field::Url => 2048,
        Field::Notes => 16384,
    };
    if value.len() > limit {
        return Err(match field {
            Field::Title => "This app or website name is too long.",
            Field::Username => "This username is too long.",
            Field::Password => "This password is too long.",
            Field::Url => "This link is too long.",
            Field::Notes => "These notes are too long.",
        }
        .into());
    }
    if field == Field::Url && !value.trim().is_empty() {
        crate::links::web_url(value)?;
    }
    Ok(())
}
fn valid_entry(entry: &Entry) -> bool {
    !entry.password.is_empty()
        && Field::ALL
            .into_iter()
            .all(|field| text_field_result(field, field.value(entry)).is_ok())
}
fn entry_title(entry: &Entry) -> String {
    let label = if !entry.title.trim().is_empty() {
        entry.title.trim()
    } else if !entry.username.trim().is_empty() {
        entry.username.trim()
    } else if !entry.url.trim().is_empty() {
        entry.url.trim()
    } else {
        "Password"
    };
    let mut end = label.len().min(256);
    while !label.is_char_boundary(end) {
        end -= 1;
    }
    label[..end].to_owned()
}

#[derive(Clone, Debug, Default)]
struct Signal {
    shared: Rc<RefCell<SignalState>>,
    seen: u64,
}
#[derive(Debug, Default)]
struct SignalState {
    revision: u64,
    waiter: Option<Waker>,
}
impl Signal {
    fn notify(&self) {
        let mut state = self.shared.borrow_mut();
        state.revision += 1;
        if let Some(waker) = state.waiter.take() {
            waker.wake();
        }
    }
}
impl Watcher for Signal {
    type Output = ();
    async fn next(&mut self) -> Option<()> {
        poll_fn(|ctx| {
            let mut state = self.shared.borrow_mut();
            if state.revision != self.seen {
                self.seen = state.revision;
                Poll::Ready(Some(()))
            } else {
                state.waiter = Some(ctx.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
    fn get(&mut self, _: &mut Context) {
        self.seen = self.shared.borrow().revision;
    }
}
#[derive(Clone, Debug)]
struct LogoWatch(SiteLogos);
impl Watcher for LogoWatch {
    type Output = ();
    async fn next(&mut self) -> Option<()> {
        let results = self.0.next().await;
        self.0.finish(results);
        Some(())
    }
    fn get(&mut self, _: &mut Context) {}
}
#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
