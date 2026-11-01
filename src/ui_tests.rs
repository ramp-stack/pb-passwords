//! End-to-end application state and rendering checks with in-memory AIR/hardware.
use super::*;
use crate::storage::tests::MemoryAir;
use prism::{Window, drawable::Item, events::WindowEvent, hardware, traits};
use prism::{air::Id, drawable::Drawable, events::Event};
use std::time::Duration;
use std::{
    marker::PhantomData,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context as TaskContext, Poll, Waker},
};

#[derive(Clone, Default)]
struct TestKeyboard(Arc<AtomicBool>);
impl traits::Keyboard for TestKeyboard {
    fn show(&self, _: f32) {
        self.0.store(true, Ordering::Relaxed);
    }
    fn hide(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
    fn visible(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Default)]
struct TestHardware {
    keyboard: TestKeyboard,
    clipboard: Arc<Mutex<Option<hardware::Data>>>,
    shared: Arc<Mutex<Vec<hardware::Data>>>,
}
struct CancelledPick<T>(PhantomData<T>);
impl<T: Clone + 'static> traits::Picked<T> for CancelledPick<T> {
    async fn get(&mut self) -> hardware::PickResult<T> {
        Err(hardware::Cancelled)
    }
}
impl<T: Clone + 'static> traits::Picker<T> for TestHardware {
    type Picked = CancelledPick<T>;
    fn pick(&self) -> Self::Picked {
        CancelledPick(PhantomData)
    }
}
impl traits::Hardware for TestHardware {
    fn keyboard(&self) -> hardware::Keyboard {
        hardware::Keyboard::new(self.keyboard.clone())
    }
    fn player(&self, _: media::Player) -> Result<hardware::Player, String> {
        panic!("unexpected media playback")
    }
    fn start_camera(&self) -> Result<hardware::Camera, String> {
        panic!("unexpected camera")
    }
    fn start_microphone(&self) -> Result<hardware::Microphone, String> {
        panic!("unexpected microphone")
    }
    fn start_capture(&self) -> Result<hardware::Capture, String> {
        panic!("unexpected capture")
    }
    fn share(&self, data: &hardware::Data) {
        self.shared.lock().unwrap().push(data.clone());
    }
    fn set_clipboard(&self, value: &hardware::Data) {
        *self.clipboard.lock().unwrap() = Some(value.clone());
    }
    fn get_clipboard(&self) -> Option<hardware::Data> {
        self.clipboard.lock().unwrap().clone()
    }
    fn set_cache(&self, _: &str, _: &hardware::Data) -> Result<(), String> {
        panic!("UI must not cache plaintext")
    }
    fn get_cache(&self, _: &str) -> Option<hardware::Data> {
        None
    }
    fn trigger_haptic(&self) {}
}

struct TestApp {
    ui: ClipboardUi,
    model: PasswordsApp,
    theme: Theme,
    ctx: Context,
    hardware: TestHardware,
}
impl TestApp {
    fn new() -> Self {
        Self::with_air(MemoryAir::default())
    }
    fn with_air(memory: MemoryAir) -> Self {
        let hardware = TestHardware::default();
        let mut ctx = Context::new(
            hardware.clone(),
            prism::air::Air::new(memory),
            Window {
                size: (1100.0, 900.0),
                is_mobile: false,
                ..Window::default()
            },
        );
        let model = PasswordsApp::new(&mut ctx);
        let theme = model.theme().to_pelican(assets());
        let ui = model.clone().build_ui(
            &mut ctx,
            Assets::new(vec![include_dir::include_dir!(
                "$CARGO_MANIFEST_DIR/resources"
            )]),
        );
        let mut app = Self {
            ui,
            model,
            theme,
            ctx,
            hardware,
        };
        app.flush();
        app
    }
    fn layout(&mut self) -> prism::drawable::LayoutTree {
        let (min, max) = self.ui.request_width();
        let width = self.ctx.window.size.0.clamp(min, max);
        let (min, max) = self.ui.request_height(width);
        let height = self.ctx.window.size.1.clamp(min, max);
        Drawable::layout(&mut self.ui, (width, height))
    }
    fn flush(&mut self) {
        let mut pending = self.ctx.events();
        let mut idle = 0;
        for _ in 0..500 {
            if let Some(event) = pending.pop_front() {
                idle = 0;
                let layout = self.layout();
                self.ui.event(&mut self.ctx, &layout, event);
                pending.extend(self.ctx.events());
                continue;
            }
            // CHK form binding and validation run through normal UI listeners.
            let update = {
                let mut future = self.ui.changed();
                future
                    .as_mut()
                    .poll(&mut TaskContext::from_waker(Waker::noop()))
            };
            match update {
                Poll::Ready(callback) => {
                    idle = 0;
                    callback(&mut self.ui as &mut dyn std::any::Any, &mut self.ctx);
                    pending.extend(self.ctx.events());
                }
                Poll::Pending if idle < 2 => {
                    idle += 1;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Poll::Pending => return,
            }
        }
        panic!("event routing failed to settle");
    }
    fn send(&mut self, event: impl Event) {
        self.ctx.emit(event);
        self.flush();
    }
    fn text(&mut self) -> String {
        let layout = self.layout();
        self.ui
            .draw(
                &layout,
                (0.0, 0.0),
                (0.0, 0.0, self.ctx.window.size.0, self.ctx.window.size.1),
            )
            .into_iter()
            .filter_map(|instruction| match instruction.item {
                Item::Text(text) => Some(
                    text.spans
                        .into_iter()
                        .map(|span| span.text)
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn clipboard(&self) -> String {
        match self.hardware.clipboard.lock().unwrap().as_ref() {
            Some(hardware::Data::Text(value)) => value.clone(),
            _ => String::new(),
        }
    }
    fn text_position(&mut self, label: &str) -> (f32, f32) {
        self.find_text(label)
            .unwrap_or_else(|| panic!("visible text not found: {label}"))
    }
    fn find_text(&mut self, label: &str) -> Option<(f32, f32)> {
        let layout = self.layout();
        self.ui
            .draw(
                &layout,
                (0.0, 0.0),
                (0.0, 0.0, self.ctx.window.size.0, self.ctx.window.size.1),
            )
            .into_iter()
            .find_map(|instruction| match instruction.item {
                Item::Text(text)
                    if text
                        .spans
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>()
                        == label =>
                {
                    let size = text.size();
                    Some((
                        instruction.offset.0 + size.0 / 2.0,
                        instruction.offset.1 + size.1 / 2.0,
                    ))
                }
                _ => None,
            })
    }
    fn scroll(&mut self, delta: f32) {
        use prism::events::{MouseEvent, MouseState};
        self.send(MouseEvent {
            position: Some((self.ctx.window.size.0 * 0.6, self.ctx.window.size.1 * 0.5)),
            state: MouseState::Scroll(0.0, delta),
        });
    }
    fn click_scrolled_text(&mut self, label: &str) {
        if self.find_text(label).is_none() {
            self.scroll(-5000.0);
            for _ in 0..12 {
                if self.find_text(label).is_some() {
                    break;
                }
                self.scroll(150.0);
            }
        }
        assert!(
            self.find_text(label).is_some(),
            "Cannot scroll to {label}: {}",
            self.text()
        );
        self.click_text(label);
    }
    fn click_text(&mut self, label: &str) {
        use prism::events::{MouseButton, MouseEvent, MouseState};
        let position = Some(self.text_position(label));
        self.send(MouseEvent {
            position,
            state: MouseState::Pressed(MouseButton::Left),
        });
        self.send(MouseEvent {
            position,
            state: MouseState::Released(MouseButton::Left),
        });
    }
    fn open(&mut self, mut flow: Flow) {
        (flow.build(&mut self.ctx, &self.theme))(&mut self.ctx, &self.theme);
        self.flush();
    }
    fn editor(&mut self, entry: Entry) -> Editor {
        if !entry.id.is_empty() {
            self.open(flow(self.model.details(entry.id.clone())));
        }
        let editor = Editor::new(entry);
        self.open(self.model.editor_with(editor.clone()));
        editor
    }
    fn set_field(&mut self, editor: &Editor, field: Field, value: &str) {
        (Action::set_text(editor.ids[field as usize], value).get())(&mut self.ctx, &self.theme);
        self.flush();
    }
    fn type_text(&mut self, text: &str) {
        use prism::events::{Key, KeyboardEvent, KeyboardState, Modifiers};
        self.send(KeyboardEvent {
            key: Key::Character(text.into()),
            state: KeyboardState::Pressed,
            modifiers: Modifiers::default(),
        });
    }
    fn click_at(&mut self, x: f32, y: f32) {
        use prism::events::{MouseButton, MouseEvent, MouseState};
        for state in [
            MouseState::Pressed(MouseButton::Left),
            MouseState::Released(MouseButton::Left),
        ] {
            self.send(MouseEvent {
                position: Some((x, y)),
                state,
            });
        }
    }
    fn header_position(&mut self, title: &str) -> (f32, f32) {
        let layout = self.layout();
        self.ui
            .draw(
                &layout,
                (0.0, 0.0),
                (0.0, 0.0, self.ctx.window.size.0, self.ctx.window.size.1),
            )
            .into_iter()
            .filter_map(|instruction| match instruction.item {
                Item::Text(text)
                    if text
                        .spans
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>()
                        == title =>
                {
                    let size = text.size();
                    Some((
                        instruction.offset.0 + size.0 / 2.0,
                        instruction.offset.1 + size.1 / 2.0,
                    ))
                }
                _ => None,
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or_else(|| panic!("header not found: {title}"))
    }
    fn header_icon(&mut self, title: &str, right: bool) {
        let (_, y) = self.header_position(title);
        let layout = self.layout();
        let mut icons = self
            .ui
            .draw(
                &layout,
                (0.0, 0.0),
                (0.0, 0.0, self.ctx.window.size.0, self.ctx.window.size.1),
            )
            .into_iter()
            .filter_map(|instruction| match instruction.item {
                Item::Svg(shape, _, _) => {
                    let point = (
                        instruction.offset.0 + shape.width / 2.0,
                        instruction.offset.1 + shape.height / 2.0,
                    );
                    ((point.1 - y).abs() < 12.0).then_some(point)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        icons.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (x, y) = if right { icons.last() } else { icons.first() }
            .copied()
            .expect("header icon");
        self.click_at(x, y);
    }
    fn back(&mut self, title: &str) {
        self.header_icon(title, false);
    }
    fn header_right(&mut self, title: &str) {
        self.header_icon(title, true);
    }
    fn entries(&self) -> Vec<Entry> {
        self.model.session.borrow_mut().entries().unwrap()
    }
    fn transfers(&self) -> std::collections::HashMap<Id, Instance<LoginTransfer>> {
        sharing::open(&self.ctx.air).list()
    }
    fn add_login(&mut self) -> String {
        self.click_text("Add password");
        for (placeholder, value) in [
            ("Enter username...", "ui@example.test"),
            ("Enter password...", "the private UI credential"),
            ("Enter link (optional)...", "https://example.test"),
            ("Enter notes (optional)...", "private UI notes"),
        ] {
            self.click_text(placeholder);
            self.type_text(value);
        }
        assert!(self.text().contains("the private UI credential"));
        self.click_text("Save");
        assert_eq!(self.entries().len(), 1);
        assert!(self.text().contains("Login created"));
        self.click_text("Done");
        assert!(self.text().contains("Add password"));
        self.entries()[0].id.clone()
    }
}
fn assets() -> Assets {
    Assets::new(vec![include_dir::include_dir!(
        "$CARGO_MANIFEST_DIR/resources"
    )])
}

#[test]
fn opens_to_saved_logins_only_and_creates_through_chk_form() {
    let mut app = TestApp::new();
    let text = app.text();
    assert!(text.contains("A fresh start. Add your first password."));
    for removed in [
        "Generate",
        "Capital letter",
        "Master password",
        "Keychain",
        "vault key",
        "Welcome",
        "AIR",
    ] {
        assert!(!text.contains(removed));
    }
    app.add_login();
    assert!(app.text().contains("ui@example.test"));
    assert!(!app.text().contains("the private UI credential"));
    app.click_text("ui@example.test");
    assert!(app.text().contains("the private UI credential"));
    assert!(app.text().contains("private UI notes"));
    assert!(!app.text().contains("Show"));
    assert!(!app.text().contains("Hide"));
    app.header_right("Login details");
    assert!(app.text().contains("Edit login"));
    app.back("Edit login");
    assert!(app.text().contains("Login details"));
    app.back("Login details");
    assert!(app.text().contains("Add password"));
}
#[test]
fn app_or_website_names_persist_through_renaming_reopening_and_sharing() {
    let memory = MemoryAir::default();
    let mut app = TestApp::with_air(memory.clone());
    let editor = app.editor(Entry::new());
    assert!(app.text().contains("App or website"));
    app.set_field(&editor, Field::Title, "  Ella's Gmail  ");
    app.set_field(&editor, Field::Username, "ella@example.test");
    app.set_field(&editor, Field::Password, "test-only credential");
    app.click_text("Save");
    app.click_text("Done");
    let saved = app.entries().remove(0);
    assert_eq!(saved.title, "Ella's Gmail");
    assert!(app.text().contains("Ella's Gmail"));
    assert!(app.text().contains("ella@example.test"));
    assert!(!app.text().contains("test-only credential"));

    let editor = app.editor(saved.clone());
    assert_eq!(editor.draft.borrow().title, "Ella's Gmail");
    app.set_field(&editor, Field::Title, "Personal Gmail");
    app.click_text("Save");
    assert!(app.text().contains("Login details"));
    assert!(app.text().contains("Personal Gmail"));
    let renamed = app.entries().remove(0);
    assert_eq!(renamed.id, saved.id);
    assert_eq!(renamed.username, saved.username);
    assert_eq!(renamed.password, saved.password);
    drop(app);

    let mut app = TestApp::with_air(memory);
    assert!(app.text().contains("Personal Gmail"));
    assert!(app.text().contains("ella@example.test"));
    app.click_text("Personal Gmail");
    app.click_text("Share");
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text(other_receiving_name().to_string()));
    app.click_text("Paste clipboard");
    app.click_text("Continue");
    assert!(app.text().contains("Confirm recipient"));
    assert!(app.text().contains("Personal Gmail"));
    assert!(app.text().contains("ella@example.test"));
    assert!(app.transfers().is_empty());
    app.click_text("Send");
    let transfer = app.transfers().into_values().next().unwrap();
    assert_eq!(transfer.pending().entry.title, "Personal Gmail");
    assert_eq!(transfer.pending().entry.username, "ella@example.test");
}
#[test]
fn generator_preserves_drafts_and_options_only_apply_on_generate() {
    for mobile in [false, true] {
        let mut app = TestApp::new();
        app.ctx.window.is_mobile = mobile;
        if mobile {
            app.ctx.window.size = (390.0, 844.0);
            app.ui = app.model.clone().build_ui(&mut app.ctx, assets());
            app.flush();
        }
        let editor = app.editor(Entry::new());
        for (field, value) in [
            (Field::Title, "Work Gmail"),
            (Field::Username, "Work"),
            (Field::Url, "example.test"),
            (Field::Notes, "keep this note"),
        ] {
            app.set_field(&editor, field, value);
        }
        app.click_text("Generate");
        let original = editor.draft.borrow().password.clone();
        assert_eq!(original.split('-').count(), 3);
        assert!(app.text().contains(&original));
        for label in ["Capital letter", "Special character", "Number"] {
            app.click_scrolled_text(label);
            assert_eq!(editor.draft.borrow().password, original);
        }
        app.click_scrolled_text("Generate");
        let generated = editor.draft.borrow().password.clone();
        assert!(generated.chars().any(|c| c.is_ascii_uppercase()));
        assert!(generated.chars().any(|c| c.is_ascii_digit()));
        assert!(generated.chars().any(|c| !c.is_alphanumeric() && c != '-'));
        assert_eq!(editor.draft.borrow().notes, "keep this note");
        assert!(app.entries().is_empty());
        app.click_text("Save");
        let saved = app.entries().remove(0);
        assert_eq!(saved.password, generated);
        assert_eq!(saved.title, "Work Gmail");
        assert_eq!(saved.username, "Work");
        assert_eq!(saved.url, "example.test");
        assert_eq!(saved.notes, "keep this note");
        app.click_text("Done");
        let editor = app.editor(saved.clone());
        app.click_text("Save");
        assert!(
            app.text().contains("Edit login"),
            "unchanged form cannot save"
        );
        app.set_field(&editor, Field::Username, "Updated work");
        app.click_scrolled_text("Capital letter");
        assert_eq!(app.entries()[0], saved);
        app.click_scrolled_text("Generate");
        app.click_text("Save");
        assert!(app.text().contains("Login details"));
        assert_eq!(app.entries().len(), 1);
        assert_eq!(app.entries()[0].id, saved.id);
        assert_eq!(app.entries()[0].username, "Updated work");
        assert_eq!(app.entries()[0].title, "Work Gmail");
    }
}
#[test]
fn validation_is_inline_and_save_requires_a_valid_password() {
    let mut app = TestApp::new();
    app.ctx.window.size = (1100.0, 1500.0);
    let editor = app.editor(Entry::new());
    app.click_text("Save");
    assert!(app.entries().is_empty());
    assert!(!app.text().contains("Enter a password."));
    app.set_field(&editor, Field::Password, "visible password");
    app.set_field(&editor, Field::Title, &"x".repeat(257));
    let y = app.text_position("This app or website name is too long.").1;
    assert!(y > app.text_position("App or website").1 && y < app.text_position("Username").1);
    app.click_text("Save");
    assert!(app.entries().is_empty());
    app.set_field(&editor, Field::Title, "Gmail");
    app.set_field(&editor, Field::Url, "not a website link");
    let y = app.text_position("Enter a valid website link.").1;
    assert!(
        y > app.text_position("Link (optional)").1 && y < app.text_position("Notes (optional)").1
    );
    app.click_text("Save");
    assert!(app.entries().is_empty());
    app.set_field(&editor, Field::Url, "example.test");
    app.set_field(&editor, Field::Password, "");
    let y = app.text_position("Enter a password.").1;
    assert!(y > app.text_position("Password").1 && y < app.text_position("Link (optional)").1);
    app.click_text("Save");
    assert!(app.entries().is_empty());
    app.set_field(&editor, Field::Password, "ready to save");
    app.click_text("Save");
    assert_eq!(app.entries().len(), 1);
    assert!(app.text().contains("Done"));
    assert!(!app.text().contains("Back to passwords"));
}
#[test]
fn mobile_uses_native_keyboard_and_back_discards_the_draft() {
    let mut app = TestApp::new();
    app.ctx.window.is_mobile = true;
    app.ctx.window.size = (390.0, 844.0);
    app.ui = app.model.clone().build_ui(&mut app.ctx, assets());
    app.flush();
    app.click_text("Add password");
    assert!(!app.text().contains("Cancel"));
    assert!(!app.text().contains("Show"));
    assert!(!app.text().contains("Hide"));
    app.click_text("Enter username...");
    app.type_text("Work");
    assert!(app.hardware.keyboard.0.load(Ordering::Relaxed));
    app.click_text("Enter password...");
    app.type_text("visible native password");
    assert!(app.hardware.keyboard.0.load(Ordering::Relaxed));
    assert!(app.text().contains("visible native password"));
    app.send(WindowEvent::Focused(false));
    assert!(app.text().contains("visible native password"));
    app.back("Add password");
    assert!(app.entries().is_empty());
    assert!(!app.hardware.keyboard.0.load(Ordering::Relaxed));
    assert!(!app.text().contains("Generate"));
}
#[test]
fn deleting_requires_confirmation_and_returns_to_the_list() {
    let mut app = TestApp::new();
    app.add_login();
    let saved = app.entries().remove(0);
    app.editor(saved);
    app.click_scrolled_text("Delete");
    assert_eq!(app.entries().len(), 1);
    assert!(app.text().contains("Delete password?"));
    app.click_text("Cancel");
    assert_eq!(app.entries().len(), 1);
    // CHK's Cancel advances out of the confirmation flow.
    app.click_scrolled_text("Delete");
    assert!(app.text().contains("Delete password?"), "{}", app.text());
    app.click_text("Delete");
    assert!(app.entries().is_empty());
    assert!(app.text().contains("Add password"));
}
#[test]
fn clipboard_expiry_preserves_a_newer_copy() {
    let mut app = TestApp::new();
    let copy = copy_action("Copy", "temporary copied password");
    app.open(flow(page(PageType::display(
        "Copy test",
        vec![Display::cta(
            "Password",
            Some("temporary copied password"),
            "",
            vec![copy.clone()],
        )],
        None,
        Bumper::None,
        Offset::Start,
    ))));
    app.click_text("Copy");
    assert_eq!(app.clipboard(), "temporary copied password");
    app.ui.expire_copy_now();
    app.flush();
    assert!(app.clipboard().is_empty());
    app.open(flow(page(PageType::display(
        "Copy test",
        vec![Display::cta(
            "Password",
            Some("temporary copied password"),
            "",
            vec![copy.clone()],
        )],
        None,
        Bumper::None,
        Offset::Start,
    ))));
    app.click_text("Copy");
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text("a newer user copy".into()));
    app.ui.expire_copy_now();
    app.flush();
    assert_eq!(app.clipboard(), "a newer user copy");
}
fn other_receiving_name() -> Name {
    "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        .parse()
        .unwrap()
}
#[test]
fn native_share_contains_only_the_public_peanut_butter_name() {
    let mut app = TestApp::new();
    app.add_login();
    app.header_right("Passwords");
    let text = app.text();
    assert!(text.contains("Peanut Butter Name"));
    assert!(!text.to_lowercase().contains("receiving name"));
    app.click_text("Copy");
    assert_eq!(app.clipboard(), app.ctx.air.me().to_string());
    app.click_text("Show QR code");
    app.click_text("Share");
    let shares = app.hardware.shared.lock().unwrap();
    assert_eq!(shares.len(), 1);
    let hardware::Data::Text(value) = &shares[0] else {
        panic!("expected public name text");
    };
    assert_eq!(value, &app.ctx.air.me().to_string());
    assert!(app.transfers().is_empty());
}
#[test]
fn sharing_requires_valid_recipient_then_explicit_send() {
    let mut app = TestApp::new();
    app.add_login();
    app.click_text("ui@example.test");
    app.click_text("Share");
    assert!(app.text().contains("Peanut Butter Name"));
    for (value, error) in [
        ("invalid name".to_owned(), "Enter a complete"),
        (app.ctx.air.me().to_string(), "Choose another"),
    ] {
        app.ctx.hardware.set_clipboard(&hardware::Data::Text(value));
        let label = if app.text().contains("Paste clipboard") {
            "Paste clipboard"
        } else {
            "Pasted"
        };
        app.click_text(label);
        assert!(
            app.text().contains(error),
            "expected {error}: {}",
            app.text()
        );
        app.click_text("Continue");
        assert!(app.transfers().is_empty());
        app.back("Peanut Butter Name");
        app.click_text("Share");
    }
    let recipient = other_receiving_name();
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text(recipient.to_string()));
    let label = if app.text().contains("Paste clipboard") {
        "Paste clipboard"
    } else {
        "Pasted"
    };
    app.click_text(label);
    app.click_text("Continue");
    assert!(app.text().contains("Confirm recipient"));
    assert!(app.transfers().is_empty());
    assert!(!app.text().contains("the private UI credential"));
    app.click_text("Send");
    assert_eq!(app.transfers().len(), 1);
    let transfer = app.transfers().into_values().next().unwrap();
    assert_eq!(transfer.pending().recipient, recipient);
    assert_eq!(
        transfer.pending().entry.password,
        "the private UI credential"
    );
    assert!(app.hardware.shared.lock().unwrap().is_empty());
    assert!(app.text().contains("Waiting for recipient"));
    app.click_text("Done");
    assert!(app.text().contains("Login details"));
}
fn incoming_fixture(memory: &MemoryAir, username: &str) -> Id {
    let mut entry = Entry::new();
    entry.title = username.into();
    entry.username = username.into();
    entry.password = "received secret".into();
    entry.url = "https://example.test".into();
    entry.notes = "received private note".into();
    memory.received(LoginTransfer {
        sender: other_receiving_name(),
        recipient: Name::orange_me(),
        entry,
        status: TransferStatus::Pending,
        created_at: 1,
    })
}
#[test]
fn incoming_login_needs_explicit_save_and_stays_visible_in_review() {
    let memory = MemoryAir::default();
    let id = incoming_fixture(&memory, "received@example.test");
    let mut app = TestApp::with_air(memory);
    assert!(!app.text().contains("received secret"));
    assert!(app.entries().is_empty());
    app.click_text("received@example.test");
    assert!(app.text().contains("received secret"));
    assert!(app.text().contains("received private note"));
    assert!(!app.text().contains("Show"));
    assert!(!app.text().contains("Hide"));
    app.click_text("Save");
    assert_eq!(app.entries().len(), 1);
    assert_eq!(
        app.transfers()[&id].pending().status,
        TransferStatus::Accepted
    );
    assert!(app.text().contains("Login created"));
    app.click_text("Done");
    assert!(!app.text().contains("Received logins"));
}
#[test]
fn declining_never_imports_a_login() {
    let memory = MemoryAir::default();
    let id = incoming_fixture(&memory, "declined@example.test");
    let mut app = TestApp::with_air(memory);
    app.click_text("declined@example.test");
    app.click_text("Decline");
    assert!(app.entries().is_empty());
    assert_eq!(
        app.transfers()[&id].pending().status,
        TransferStatus::Declined
    );
    assert!(app.text().contains("Add password"));
}
#[test]
fn incoming_waits_for_confirmed_private_save() {
    let memory = MemoryAir::default();
    let id = incoming_fixture(&memory, "delayed@example.test");
    let mut app = TestApp::with_air(memory.clone());
    memory.confirm_private(false);
    app.click_text("delayed@example.test");
    app.click_text("Save");
    assert!(app.text().contains("Saving login…"));
    assert!(!app.text().contains("Decline"));
    assert_eq!(
        app.transfers()[&id].pending().status,
        TransferStatus::Pending
    );
    assert_eq!(app.entries().len(), 1);
    memory.confirm_private(true);
    // MemoryAir has no asynchronous notifications; reopen to deliver a fresh snapshot.
    app.back("Received login");
    app.click_text("delayed@example.test");
    assert!(app.text().contains("Login created"));
    assert_eq!(
        app.transfers()[&id].pending().status,
        TransferStatus::Accepted
    );
    assert_eq!(app.entries().len(), 1);
}

#[test]
fn contacts_remember_recipients_and_allow_sharing_again_from_either_page() {
    let mut app = TestApp::new();
    app.add_login();
    app.click_text("Contacts");
    assert!(
        app.text()
            .contains("Add a contact or share a login to get started.")
    );
    assert!(!app.text().contains("the private UI credential"));
    app.click_text("Passwords");
    app.click_text("ui@example.test");
    app.click_text("Share");
    let recipient = other_receiving_name();
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text(recipient.to_string()));
    app.click_text("Paste clipboard");
    app.click_text("Continue");
    assert!(app.transfers().is_empty());
    app.click_text("Send");
    app.click_text("Done");
    app.click_text("Contacts");
    assert!(app.text().contains("Shared once"));
    assert_eq!(app.text().matches(&short_name(recipient)).count(), 1);
    app.click_text(&short_name(recipient));
    let text = app.text();
    assert!(text.contains(&recipient.to_string()));
    assert!(text.contains("Shared logins"));
    assert!(text.contains("ui@example.test"));
    assert!(!text.contains("the private UI credential"));
    assert!(!text.contains("private UI notes"));
    app.click_text("Share a login");
    assert!(app.text().contains("Choose login"));
    app.click_text("ui@example.test");
    assert!(app.text().contains("Confirm recipient"));
    assert!(app.text().contains(&recipient.to_string()));
    assert_eq!(
        app.transfers().len(),
        1,
        "choosing a contact and login must not send"
    );
    app.back("Confirm recipient");
    app.back("Choose login");
    assert!(app.text().contains("Shared logins"));
    app.click_text("Share a login");
    app.click_text("ui@example.test");
    app.click_text("Send");
    assert_eq!(app.transfers().len(), 2);
    app.click_text("Contacts");
    assert!(app.text().contains("Shared 2 times"));
    assert_eq!(app.text().matches(&short_name(recipient)).count(), 1);
    app.click_text("Passwords");
    app.click_text("ui@example.test");
    app.click_text("Share");
    assert!(app.text().contains("Someone new"));
    assert!(app.text().contains(&short_name(recipient)));
    app.click_text(&short_name(recipient));
    assert!(app.text().contains("Confirm recipient"));
    assert_eq!(app.transfers().len(), 2);
    app.back("Confirm recipient");
    app.click_text("Someone new");
    assert!(app.text().contains("Scan QR code"));
    assert_eq!(app.transfers().len(), 2);
    // The same AIR history restores the contacts after restarting the app.
    app.model = PasswordsApp::new(&mut app.ctx);
    app.ui = app.model.clone().build_ui(&mut app.ctx, assets());
    app.flush();
    app.click_text("Contacts");
    assert!(app.text().contains("Shared 2 times"));
    assert_eq!(app.text().matches(&short_name(recipient)).count(), 1);
}
#[test]
fn incoming_only_people_do_not_appear_as_contacts() {
    let memory = MemoryAir::default();
    incoming_fixture(&memory, "unsolicited@example.test");
    let mut app = TestApp::with_air(memory);
    app.click_text("Contacts");
    assert!(
        app.text()
            .contains("Add a contact or share a login to get started.")
    );
    assert!(!app.text().contains(&short_name(other_receiving_name())));
}

#[test]
fn saved_contact_can_be_added_without_a_login_and_reused_after_reopening() {
    let mut app = TestApp::new();
    let name = other_receiving_name();
    app.click_text("Contacts");
    app.click_text("Add contact");
    assert!(app.text().contains("Scan QR code"));
    assert!(app.text().contains("Enter peanut butter name..."));
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text(format!("  {name}\n")));
    app.click_text("Paste clipboard");
    app.click_text("Continue");
    assert!(app.text().contains("Contact name"));
    assert!(app.text().contains(&name.to_string()));
    assert!(app.text().contains("No shared logins yet."));
    assert!(app.entries().is_empty());
    assert!(app.transfers().is_empty());
    let profiles = app.ctx.air.instances::<Profile>().list();
    assert_eq!(profiles.len(), 1);
    let username = profiles
        .into_values()
        .next()
        .unwrap()
        .pending()
        .username
        .clone();
    app.click_text("Share a login");
    assert!(
        app.text()
            .contains("Add a password before sharing a login.")
    );
    app.back("Choose login");
    app.header_right("Contact");
    let layout = app.layout();
    let input_id = app
        .ui
        .draw(&layout, (0.0, 0.0), (0.0, 0.0, 1100.0, 900.0))
        .into_iter()
        .find_map(|instruction| match instruction.item {
            Item::Text(text)
                if text
                    .spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<String>()
                    == username =>
            {
                Some(text.id)
            }
            _ => None,
        })
        .expect("contact name input");
    (Action::set_text(input_id, "Alex").get())(&mut app.ctx, &app.theme);
    app.flush();
    app.click_text("Save");
    assert!(app.text().contains("Alex"));
    app.back("Contact");
    assert!(app.text().contains("No logins shared"));
    assert_eq!(app.text().matches("Alex").count(), 1);
    // Adding the same Peanut Butter Name again opens the saved contact and preserves its label.
    app.click_text("Add contact");
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text(name.to_string()));
    app.click_text("Paste clipboard");
    app.click_text("Continue");
    assert_eq!(app.ctx.air.instances::<Profile>().list().len(), 1);
    assert!(app.text().contains("Alex"));
    assert!(app.transfers().is_empty());
    app.model = PasswordsApp::new(&mut app.ctx);
    app.ui = app.model.clone().build_ui(&mut app.ctx, assets());
    app.flush();
    app.click_text("Contacts");
    assert!(app.text().contains("Alex"));
    app.click_text("Passwords");
    app.add_login();
    app.click_text("ui@example.test");
    app.click_text("Share");
    assert!(app.text().contains("Alex"));
    app.click_text("Alex");
    assert!(app.text().contains("Confirm recipient"));
    assert!(app.text().contains(&name.to_string()));
    assert!(app.transfers().is_empty());
    app.click_text("Send");
    assert_eq!(app.transfers().len(), 1);
    app.click_text("Contacts");
    assert_eq!(app.text().matches("Alex").count(), 1);
    assert!(app.text().contains("Shared once"));
    assert!(app.hardware.shared.lock().unwrap().is_empty());
}

#[test]
fn add_contact_rejects_invalid_and_self_names_and_back_does_not_save() {
    let mut app = TestApp::new();
    app.click_text("Contacts");
    for (value, error) in [
        (
            "not a name".to_owned(),
            "Enter a complete Peanut Butter Name.",
        ),
        (
            app.ctx.air.me().to_string(),
            "Choose another person’s Peanut Butter Name.",
        ),
    ] {
        app.click_text("Add contact");
        app.click_text("Continue");
        assert!(app.ctx.air.instances::<Profile>().list().is_empty());
        app.ctx.hardware.set_clipboard(&hardware::Data::Text(value));
        app.click_text("Paste clipboard");
        assert!(app.text().contains(error));
        assert!(app.text_position(error).1 > app.text_position("Add contact").1);
        app.click_text("Continue");
        assert!(app.ctx.air.instances::<Profile>().list().is_empty());
        assert!(app.transfers().is_empty());
        app.back("Add contact");
    }
    app.click_text("Add contact");
    app.click_text("Enter peanut butter name...");
    app.type_text(&other_receiving_name().to_string());
    app.back("Add contact");
    assert!(app.ctx.air.instances::<Profile>().list().is_empty());
    assert!(
        app.text()
            .contains("Add a contact or share a login to get started.")
    );
}
