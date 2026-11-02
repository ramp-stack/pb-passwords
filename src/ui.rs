//! All pages, inputs, navigation, and controls are composed through CHK.
use crate::{
    contacts,
    passphrase::{PassphraseOptions, generate_passphrase},
    requests::{self, LoginRequest, RequestStatus},
    runtime::{ClipboardUi, copy_action},
    sharing::{self, LoginTransfer, TransferStatus},
    site_logos::SiteLogos,
    storage::{Entry, PasswordStore},
};
use chk::profiles::Profile;
use chk::{
    Action, ActionItem, App, Assets, AvatarContent, AvatarIconStyle, Bumper, ChkTheme, Color,
    Context, Display, Flow, Form, FormAction, FormComplete, FormItem, FormValidState, Icons, Input,
    ListItem, Offset, Page, PageType, Root, RootInfo, Selector, State, Theme, Watch, Watcher,
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
enum ShareFrom {
    Login(String),
    Contact(Name),
}

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
        requests::open(&ctx.air);
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
                (
                    Watch::list::<LoginTransfer>(),
                    Watch::list::<LoginRequest>(),
                ),
                LogoWatch(self.logos.clone()),
                self.changes.clone(),
            ),
            move |ctx: &mut Context, theme: &Theme, (_, (transfers, requests), _, ())| {
                let mut items = vec![];
                if let Err(error) =
                    sharing::reconcile_received(ctx, &mut app.session.borrow_mut(), &transfers)
                {
                    items.push(Display::text(&error));
                }
                if let Err(error) =
                    requests::reconcile_received(ctx, &mut app.session.borrow_mut(), &requests)
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
                let search_app = app.clone();
                let editor = app.clone();
                PageType::root(
                    "Passwords",
                    vec![],
                    items,
                    Some((
                        Icons::Search,
                        Box::new(move |_: &mut Context, theme: &Theme| {
                            search_app.search_passwords(theme)
                        }),
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
                        let mut actions = vec![
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
                        ];
                        actions.extend(editor.option_actions());
                        actions
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
                let mut display = vec![];
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
    fn contacts(&self) -> Page {
        let app = self.clone();
        Page::watching(
            (
                Watch::list::<LoginTransfer>(),
                Watch::list::<Profile>(),
                Watch::list::<LoginRequest>(),
                self.changes.clone(),
            ),
            move |_: &mut Context, _: &Theme, (transfers, profiles, requests, ())| {
                let contacts = contacts::with_requests(app.name, &transfers, &profiles, &requests);
                let rows = contacts
                    .iter()
                    .map(|contact| {
                        let next = app.contact(contact.name);
                        let profile = profiles
                            .iter()
                            .find(|profile| profile.pending().name == Some(contact.name));
                        let (avatar, title, subtitle) = match profile {
                            Some(profile) => {
                                let profile = profile.pending();
                                (
                                    profile.avatar.clone(),
                                    profile.username.clone(),
                                    short_name(contact.name),
                                )
                            }
                            None => (
                                AvatarContent::icon(Icons::Profile, AvatarIconStyle::Brand),
                                short_name(contact.name),
                                String::new(),
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
                let name = app.name;
                let app = app.clone();
                PageType::root(
                    "Contacts",
                    vec![],
                    items,
                    Some((
                        Icons::QrCode,
                        Box::new(move |_: &mut Context, _: &Theme| flow(name_page(name))),
                    )),
                    Some((
                        "Add contact".into(),
                        Box::new(move |_: &mut Context, theme: &Theme| app.add_contact(theme)),
                    )),
                    None,
                )
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
        let selector = Selector::new("contact name", self.contact_items(ctx));
        let actions = vec![FormAction::Custom(
            "New contact".into(),
            None,
            Icons::Add,
            Action::flow(self.add_share_contact(theme, selector.clone())),
        )];
        self.share_picker_flow(
            theme,
            FormItem::selection("Share login", selector.with_actions(actions)),
            ShareFrom::Login(id),
        )
    }

    fn contact_items(&self, ctx: &mut Context) -> Vec<(ListItem, String)> {
        let transfers: Vec<_> = sharing::open(&ctx.air).list().into_values().collect();
        let profiles: Vec<_> = ctx
            .air
            .instances::<Profile>()
            .list()
            .into_values()
            .collect();
        let requests: Vec<_> = requests::open(&ctx.air).list().into_values().collect();
        contacts::with_requests(self.name, &transfers, &profiles, &requests)
            .into_iter()
            .map(|contact| {
                let profile = profiles
                    .iter()
                    .find(|profile| profile.pending().name == Some(contact.name));
                let (avatar, title) = match profile {
                    Some(profile) => {
                        let profile = profile.pending();
                        (profile.avatar.clone(), profile.username.clone())
                    }
                    None => (
                        AvatarContent::icon(Icons::Profile, AvatarIconStyle::Brand),
                        short_name(contact.name),
                    ),
                };
                (
                    ListItem::avatar(avatar, &title, &contact.name.to_string(), None, None),
                    contact.name.to_string(),
                )
            })
            .collect()
    }
    fn selected_contacts(&self, keys: &[String]) -> Option<Vec<Name>> {
        let recipients = keys
            .iter()
            .map(|key| sharing::recipient(key, self.name))
            .collect::<Result<std::collections::BTreeSet<_>, _>>()
            .ok()?;
        (!recipients.is_empty()).then(|| recipients.into_iter().collect())
    }
    fn add_share_contact(&self, theme: &Theme, selector: Selector) -> Flow {
        let app = self.clone();
        let me = self.name;
        Flow::from_form(Form::flow(
            theme,
            vec![FormItem::text(
                "New contact",
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
                let profile = Profile::from_name(ctx, name);
                let profile = profile.pending();
                let row = ListItem::avatar(
                    profile.avatar.clone(),
                    &profile.username,
                    &name.to_string(),
                    None,
                    None,
                );
                let selection = selector.select(row, name.to_string());
                app.changes.notify();
                FormComplete::Return(Box::new(move |ctx: &mut Context, theme: &Theme| {
                    (selection.get())(ctx, theme)
                }))
            }),
        ))
    }
    fn contact(&self, name: Name) -> Page {
        let app = self.clone();
        Page::watching(
            (Watch::list::<Profile>(), self.changes.clone()),
            move |ctx: &mut Context, _: &Theme, (profiles, ())| {
                let profile = profiles
                    .into_iter()
                    .find(|profile| profile.pending().name == Some(name))
                    .unwrap_or_else(|| Profile::from_name(ctx, name));
                // Reuse Orange's CHK profile page, including its avatar, bio, and editor.
                let mut profile_page = chk::ProfileView::build(ctx, profile);
                if let PageType::Display { items, .. } = &mut profile_page {
                    let share_app = app.clone();
                    let request_app = app.clone();
                    for item in items.iter_mut() {
                        match item {
                            Display::Actions { .. } => {
                                *item = Display::cta(
                                    "Passwords",
                                    None,
                                    "",
                                    vec![
                                        ActionItem::new(
                                            Action::custom({
                                                let app = share_app.clone();
                                                move |ctx: &mut Context, theme: &Theme| {
                                                    let mut next = app.choose_logins(name, theme);
                                                    (next.build(ctx, theme))(ctx, theme);
                                                }
                                            }),
                                            "Share",
                                            Icons::Send,
                                        ),
                                        ActionItem::new(
                                            Action::custom({
                                                let app = request_app.clone();
                                                move |ctx: &mut Context, theme: &Theme| {
                                                    let mut next =
                                                        app.request_password(name, theme);
                                                    (next.build(ctx, theme))(ctx, theme);
                                                }
                                            }),
                                            "Request",
                                            Icons::Credential,
                                        ),
                                    ],
                                );
                            }
                            Display::Cta { label, .. } if label == "Orange name" => {
                                *item = Display::cta(
                                    "Peanut Butter Name",
                                    None,
                                    &name.to_string(),
                                    vec![
                                        copy_action("Copy", &name.to_string()),
                                        ActionItem::new(
                                            Action::flow(flow(page(PageType::display_qr_code(
                                                "Peanut Butter Name",
                                                &name.to_string(),
                                                "",
                                                true,
                                            )))),
                                            "Show QR code",
                                            Icons::QrCode,
                                        ),
                                    ],
                                );
                            }
                            _ => {}
                        }
                    }
                }
                profile_page
            },
        )
    }
    fn login_selector(&self, theme: &Theme) -> Result<Selector, String> {
        let entries = self
            .session
            .borrow_mut()
            .entries()
            .map_err(|error| error.to_string())?;
        if entries.is_empty() {
            return Err("Add a password to get started.".into());
        }
        let items = entries
            .iter()
            .map(|entry| {
                (
                    ListItem::avatar(
                        self.logos
                            .get(label(entry), &entry.url)
                            .unwrap_or_else(|| login_avatar(theme)),
                        label(entry),
                        login_subtitle(entry),
                        None,
                        None,
                    ),
                    entry.id.clone(),
                )
            })
            .collect();
        Ok(Selector::new("passwords", items)
            .with_limit(requests::MAX_RESPONSES)
            .with_chip_subtitles(true))
    }
    fn search_passwords(&self, theme: &Theme) -> Flow {
        let selector = match self.login_selector(theme) {
            Ok(selector) => selector.with_limit(1),
            Err(error) => return flow(page(message("Search passwords", &error))),
        };
        let app = self.clone();
        Flow::from_form(Form::flow(
            theme,
            vec![FormItem::selection("Search passwords", selector)],
            None,
            None,
            Box::new(move |_: &mut Context, values: &Vec<State>| {
                let Some(State::Selection(ids)) = values.first() else {
                    return FormComplete::None;
                };
                let [id] = ids.as_slice() else {
                    return FormComplete::None;
                };
                FormComplete::Next(app.details(id.clone()))
            }),
        ))
    }
    fn choose_logins(&self, recipient: Name, theme: &Theme) -> Flow {
        let selector = match self.login_selector(theme) {
            Ok(selector) => selector,
            Err(error) => return flow(page(message("Choose passwords", &error))),
        };
        self.share_picker_flow(
            theme,
            FormItem::selection("Choose passwords", selector),
            ShareFrom::Contact(recipient),
        )
    }
    fn share_picker_flow(&self, theme: &Theme, picker: FormItem, source: ShareFrom) -> Flow {
        let selected = Rc::new(RefCell::new(Vec::<String>::new()));
        let update = Signal::default();
        let values = selected.clone();
        let signal = update.clone();
        let mut pages = Flow::from_form(Form::flow(
            theme,
            vec![picker],
            None,
            None,
            Box::new(move |_: &mut Context, values_in: &Vec<State>| {
                if let Some(State::Selection(keys)) = values_in.first() {
                    *values.borrow_mut() = keys.clone();
                    signal.notify();
                }
                FormComplete::None
            }),
        ));
        let app = self.clone();
        pages.0.push(Page::watching(
            (
                update,
                LogoWatch(self.logos.clone()),
                Watch::list::<Profile>(),
                Watch::contract(self.session.borrow().instance()),
            ),
            move |ctx: &mut Context, theme: &Theme, _| {
                let keys = selected.borrow().clone();
                let (ids, recipients) = match &source {
                    ShareFrom::Login(id) => {
                        let Some(recipients) = app.selected_contacts(&keys) else {
                            return message("Share passwords", "Choose a valid contact.");
                        };
                        (vec![id.clone()], recipients)
                    }
                    ShareFrom::Contact(name) => (keys, vec![*name]),
                };
                app.share_review(ctx, theme, ids, recipients)
            },
        ));
        pages
    }

    fn selected_entries(&self, ids: &[String]) -> Result<Vec<Entry>, String> {
        let ids = ids.iter().collect::<std::collections::BTreeSet<_>>();
        if ids.is_empty() || ids.len() > requests::MAX_RESPONSES {
            return Err("Choose between 1 and 32 logins.".into());
        }
        ids.into_iter()
            .map(|id| {
                let entry = self.entry(id)?;
                entry.validate().map_err(|error| error.to_string())?;
                Ok(entry)
            })
            .collect()
    }
    fn share_review(
        &self,
        ctx: &mut Context,
        theme: &Theme,
        ids: Vec<String>,
        recipients: Vec<Name>,
    ) -> PageType {
        let recipients: Vec<_> = recipients
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if recipients.is_empty() || recipients.contains(&self.name) {
            return message("Share passwords", "Choose another person to share with.");
        }
        let entries = match self.selected_entries(&ids) {
            Ok(entries) => entries,
            Err(error) => return message("Share passwords", &error),
        };
        let profiles = ctx
            .air
            .instances::<Profile>()
            .list()
            .into_values()
            .collect::<Vec<_>>();
        let labels = recipients
            .iter()
            .map(|name| {
                let label = profiles
                    .iter()
                    .find_map(|profile| {
                        let profile = profile.pending();
                        (profile.name == Some(*name)).then(|| profile.username.clone())
                    })
                    .filter(|label| !label.trim().is_empty())
                    .unwrap_or_else(|| short_name(*name));
                (label, *name)
            })
            .collect::<Vec<_>>();
        let to = labels
            .iter()
            .map(|(label, name)| {
                if labels.iter().filter(|(other, _)| other == label).count() > 1 {
                    format!("{label} ({})", short_name(*name))
                } else {
                    label.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let app = self.clone();
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
                    Some(flow(page(PageType::display(
                        "Login details",
                        entry_items(entry, false),
                        None,
                        Bumper::Done,
                        Offset::Start,
                    )))),
                )
            })
            .collect();
        let app = app.clone();
        let ids = ids.clone();
        let recipients = recipients.clone();
        PageType::display(
            "Share passwords",
            vec![
                Display::cta("To", Some(&to), "", vec![]),
                Display::list(Some("Logins"), rows, None),
            ],
            None,
            Bumper::custom(
                "Send",
                Action::custom(move |ctx: &mut Context, theme: &Theme| {
                    // Validate every current entry before queuing any copies.
                    let result = app.selected_entries(&ids).and_then(|entries| {
                        let mut sent = vec![];
                        for recipient in &recipients {
                            for entry in &entries {
                                sent.push(sharing::send(ctx, entry.clone(), *recipient)?);
                            }
                        }
                        Ok(sent)
                    });
                    let next = match result {
                        Ok(sent) => {
                            app.changes.notify();
                            share_sent(sent)
                        }
                        Err(error) => page(message("Share passwords", &error)),
                    };
                    FormComplete::Next(next).run(ctx, theme);
                }),
            ),
            Offset::Start,
        )
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
                            return saved_success_type("Login saved");
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
                                Ok(()) => app.changes.notify(),
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
                Root::custom(self.contacts()),
            ),
            RootInfo::icon(
                ctx,
                theme,
                Icons::Send,
                "Requests",
                Root::custom(self.requests_page()),
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
fn saved_success_type(title: &str) -> PageType {
    PageType::display(
        title,
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
    page(saved_success_type("Login created"))
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
        "Peanut Butter Name",
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
fn shared_success_type() -> PageType {
    PageType::display(
        "",
        vec![
            Display::image("illustrations/passwords-shared", (180.0, 180.0)),
            Display::confirmation_message("Shared!"),
        ],
        None,
        Bumper::Done,
        Offset::Center,
    )
}
fn share_sent(sent: Vec<Instance<LoginTransfer>>) -> Page {
    let ids = sent.iter().map(Instance::id).collect::<Vec<_>>();
    Page::watching(
        Watch::list::<LoginTransfer>(),
        move |_: &mut Context, _: &Theme, transfers: Vec<Instance<LoginTransfer>>| {
            let confirmed = !ids.is_empty()
                && ids.iter().all(|id| {
                    transfers
                        .iter()
                        .find(|transfer| transfer.id() == *id)
                        .is_some_and(|transfer| transfer.confirmed().is_some())
                });
            if confirmed {
                shared_success_type()
            } else {
                PageType::display(
                    "",
                    vec![Display::instructions("Sharing…")],
                    None,
                    Bumper::Done,
                    Offset::Center,
                )
            }
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
    fn option_actions(&self) -> Vec<chk::InputAction> {
        let options = *self.options.borrow();
        [
            ("Capital letter", options.capital),
            ("Special character", options.special),
            ("Number", options.number),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (label, selected))| {
            let options = self.options.clone();
            let signal = self.signal.clone();
            (
                label.into(),
                None,
                if selected {
                    Icons::Check
                } else {
                    Icons::Unchecked
                },
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
            )
        })
        .collect()
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
#[path = "request_ui.rs"]
mod request_pages;

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
