//! End-to-end application state and rendering checks with in-memory AIR/hardware.
use super::*;
use crate::storage::tests::MemoryAir;
use chk::profiles::ProfileAction;
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
    fn assert_shared_success(&mut self) {
        let text = self.text();
        assert!(text.contains("Shared!"));
        assert!(self.find_text("Done").is_some());
        for old in [
            "Waiting for recipient",
            "Recipients",
            "Share passwords",
            "Sharing…",
        ] {
            assert!(!text.contains(old));
        }
        let illustration = self.theme.brand().images["illustrations/passwords-shared"].clone();
        let layout = self.layout();
        assert!(
            self.ui
                .draw(
                    &layout,
                    (0.0, 0.0),
                    (0.0, 0.0, self.ctx.window.size.0, self.ctx.window.size.1)
                )
                .into_iter()
                .any(|instruction| matches!(instruction.item,
                Item::Image(_, image, _) if image == illustration))
        );
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
    fn text_input_id(&mut self, value: &str) -> uuid::Uuid {
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
                        == value =>
                {
                    Some(text.id)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("visible input not found: {value}"))
    }
    fn set_visible_input(&mut self, current: &str, value: &str) {
        let id = self.text_input_id(current);
        (Action::set_text(id, value).get())(&mut self.ctx, &self.theme);
        self.flush();
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
            .unwrap_or_else(|| panic!("header icon for {title} at y={y}"));
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
    fn requests(&self) -> std::collections::HashMap<Id, Instance<LoginRequest>> {
        requests::open(&self.ctx.air).list()
    }
    fn reopen(&mut self) {
        self.model = PasswordsApp::new(&mut self.ctx);
        self.ui = self.model.clone().build_ui(&mut self.ctx, assets());
        self.flush();
    }
    fn open_requests_tab(&mut self) {
        if !self.ctx.window.is_mobile {
            self.click_text("Requests");
            return;
        }
        // Mobile navigation has icon-only tabs; Requests is the rightmost one.
        let layout = self.layout();
        let tab = self
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
                    (point.1 > self.ctx.window.size.1 - 80.0).then_some(point)
                }
                _ => None,
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .expect("Requests tab icon");
        self.click_at(tab.0, tab.1);
    }
    fn add_login(&mut self) -> String {
        self.click_text("Add Password");
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
        assert!(self.text().contains("Add Password"));
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
        "Capital Letter",
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
    let detail = app.text();
    assert_eq!(detail.lines().filter(|line| *line == "Link").count(), 1);
    assert_eq!(detail.matches("https://example.test").count(), 1);
    assert!(!app.text().contains("Show"));
    assert!(!app.text().contains("Hide"));
    app.header_right("Login details");
    assert!(app.text().contains("Edit login"));
    app.back("Edit login");
    assert!(app.text().contains("Login details"));
    app.back("Login details");
    assert!(app.text().contains("Add Password"));
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
    app.click_text("New Contact");
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text(other_receiving_name().to_string()));
    app.click_text("Paste Clipboard");
    app.click_text("Continue");
    assert!(app.text().contains("Share login"));
    app.click_text("Continue");
    assert!(app.text().contains("Share passwords"));
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
        for label in ["Capital Letter", "Special Character", "Number"] {
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
        app.click_scrolled_text("Capital Letter");
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
fn generator_options_render_directly_beneath_password_before_optional_fields() {
    for mobile in [false, true] {
        let mut app = TestApp::new();
        app.ctx.window.is_mobile = mobile;
        app.ctx.window.size = if mobile {
            (390.0, 844.0)
        } else {
            (1100.0, 900.0)
        };
        app.reopen();
        let editor = app.editor(Entry::new());
        let password_y = app.text_position("Password").1;
        let link_y = app.text_position("Link (optional)").1;
        let notes_y = app.text_position("Notes (optional)").1;
        for label in [
            "Paste",
            "Generate",
            "Capital Letter",
            "Special Character",
            "Number",
        ] {
            let y = app.text_position(label).1;
            assert!(
                y > password_y && y < link_y,
                "{label} must follow Password before Link (mobile={mobile})"
            );
        }
        assert!(link_y < notes_y);
        app.set_field(&editor, Field::Password, "keep my typed password");
        app.set_field(&editor, Field::Url, "example.test");
        app.set_field(&editor, Field::Notes, "keep my note");
        app.click_scrolled_text("Capital Letter");
        assert_eq!(editor.draft.borrow().password, "keep my typed password");
        assert_eq!(editor.draft.borrow().url, "example.test");
        assert_eq!(editor.draft.borrow().notes, "keep my note");
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
    app.click_text("Add Password");
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
    assert!(app.text().contains("Add Password"));
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
    app.click_text("Contacts");
    app.header_right("Contacts");
    let text = app.text();
    assert!(text.contains("Peanut Butter Name"));
    assert!(!text.to_lowercase().contains("receiving name"));
    app.click_text("Copy");
    assert_eq!(app.clipboard(), app.ctx.air.me().to_string());
    app.click_text("Show QR Code");
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
    assert!(app.text().contains("Share login"));
    app.click_text("New Contact");
    assert!(app.text().contains("Enter peanut butter name..."));
    for (value, error) in [
        ("invalid name".to_owned(), "Enter a complete"),
        (app.ctx.air.me().to_string(), "Choose another"),
    ] {
        app.ctx.hardware.set_clipboard(&hardware::Data::Text(value));
        let label = if app.text().contains("Paste Clipboard") {
            "Paste Clipboard"
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
        app.back("New contact");
        app.click_text("New Contact");
    }
    let recipient = other_receiving_name();
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text(recipient.to_string()));
    let label = if app.text().contains("Paste Clipboard") {
        "Paste Clipboard"
    } else {
        "Pasted"
    };
    app.click_text(label);
    app.click_text("Continue");
    assert!(app.text().contains("Share login"));
    assert!(!app.text().contains("Share passwords"));
    assert!(app.transfers().is_empty());
    assert_eq!(app.ctx.air.instances::<Profile>().list().len(), 1);
    app.click_text("Continue");
    assert!(app.text().contains("Share passwords"));
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
    app.assert_shared_success();
    assert!(!app.text().contains("Waiting for recipient"));
    assert!(!app.text().contains(&recipient.to_string()));
    app.click_text("Done");
    assert!(app.text().contains("Login details"));
}

#[test]
fn share_contact_search_filters_matches_and_selects_multiple_recipients() {
    let mut app = TestApp::new();
    let mom = other_receiving_name();
    let alex: Name = "0379be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        .parse()
        .unwrap();
    Profile::from_name(&mut app.ctx, mom).send(ProfileAction::ChangeUsername("Mom".into()));
    Profile::from_name(&mut app.ctx, alex).send(ProfileAction::ChangeUsername("Alex".into()));
    Profile::from_name(&mut app.ctx, Name::orange_me())
        .send(ProfileAction::ChangeUsername("My own profile".into()));
    app.reopen();
    app.add_login();
    app.click_text("ui@example.test");
    app.click_text("Share");
    assert!(app.text().contains("Search contact name..."));
    assert!(!app.text().contains("My own profile"));
    app.click_text("Continue");
    assert!(app.text().contains("Share login"));
    assert!(app.transfers().is_empty());

    app.click_text("Search contact name...");
    app.type_text("m");
    let search_id = app.text_input_id("m");
    // Filtering must keep the same focused input while each character arrives.
    app.type_text("om");
    assert_eq!(app.text_input_id("mom"), search_id);
    assert!(app.find_text("Mom").is_some());
    assert!(app.find_text("Alex").is_none());
    app.click_text("Mom");
    assert_eq!(app.text_input_id("mom"), search_id);
    assert!(!app.text().contains("Share passwords"));
    assert!(app.transfers().is_empty());
    // Clicking the selected chip restores the row and disables Continue again.
    app.click_text("Mom");
    assert_eq!(app.text_input_id("mom"), search_id);
    app.click_text("Continue");
    assert!(app.text().contains("Share login"));
    assert!(!app.text().contains("Share passwords"));
    app.click_text("Mom");
    app.set_visible_input("mom", "");
    assert!(app.find_text("Alex").is_some());
    app.click_text("Alex");
    assert!(app.transfers().is_empty());
    app.click_text("Continue");
    let confirmation = app.text();
    assert!(confirmation.contains("Share passwords"));
    assert!(confirmation.contains("Mom"));
    assert!(confirmation.contains("Alex"));
    assert!(!confirmation.contains(&mom.to_string()));
    assert!(!confirmation.contains(&alex.to_string()));
    assert!(!confirmation.contains("Sends a copy"));
    assert!(!confirmation.contains("the private UI credential"));
    assert!(
        app.transfers().is_empty(),
        "selecting contacts must not send"
    );
    app.click_text("Send");
    let transfers = app.transfers();
    assert_eq!(transfers.len(), 2);
    let recipients = transfers
        .values()
        .map(|transfer| transfer.pending().recipient)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(recipients, [mom, alex].into_iter().collect());
    assert!(
        transfers
            .values()
            .all(|transfer| { transfer.pending().entry.password == "the private UI credential" })
    );
    assert!(app.hardware.shared.lock().unwrap().is_empty());
    app.assert_shared_success();
    app.click_text("Done");
    assert!(app.text().contains("Login details"));
}

#[test]
fn contact_sharing_selects_multiple_logins_and_done_returns_to_contact() {
    let mut app = TestApp::new();
    let mom = other_receiving_name();
    Profile::from_name(&mut app.ctx, mom).send(ProfileAction::ChangeUsername("Mom".into()));
    for entry in [
        requested_login("Amazon", "personal@example.test", "personal contact secret"),
        requested_login("Amazon", "family@example.test", "family contact secret"),
        requested_login("Netflix", "stream@example.test", "stream contact secret"),
    ] {
        app.model.session.borrow_mut().save(entry).unwrap();
    }
    app.reopen();
    app.click_text("Contacts");
    app.click_text("Mom");
    app.click_scrolled_text("Share");
    assert!(app.text().contains("Choose passwords"));
    app.click_text("Continue");
    assert!(
        app.text().contains("Choose passwords"),
        "Empty Continue advanced: {}",
        app.text()
    );
    assert!(app.transfers().is_empty());
    app.click_text("Search passwords...");
    app.type_text("amazon");
    assert!(app.find_text("Netflix").is_none());
    app.click_text("personal@example.test");
    app.click_text("family@example.test");
    assert!(app.find_text("Amazon · personal@example.test").is_some());
    assert!(app.find_text("Amazon · family@example.test").is_some());
    app.click_text("Continue");
    let review = app.text();
    assert!(review.contains("Share passwords"));
    assert!(review.contains("Mom"));
    assert!(review.contains("To"));
    assert!(review.contains("Logins"));
    assert!(review.contains("personal@example.test"));
    assert!(review.contains("family@example.test"));
    assert!(!review.contains(&mom.to_string()));
    assert!(!review.contains("Sends a copy"));
    assert!(!review.contains("contact secret"));
    assert!(!review.contains("Netflix"));
    assert!(app.transfers().is_empty());
    // Reviewing a selected login does not send; its Done returns to the review.
    app.click_text("personal@example.test");
    assert!(app.text().contains("Login details"));
    assert!(app.text().contains("personal contact secret"));
    assert!(app.transfers().is_empty());
    app.click_text("Done");
    assert!(app.text().contains("Share passwords"));
    assert!(!app.text().contains("personal contact secret"));
    app.click_text("Send");
    let transfers = app.transfers();
    assert_eq!(transfers.len(), 2);
    assert!(
        transfers
            .values()
            .all(|transfer| transfer.pending().recipient == mom)
    );
    let usernames = transfers
        .values()
        .map(|transfer| transfer.pending().entry.username.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        usernames,
        [
            "personal@example.test".to_owned(),
            "family@example.test".to_owned()
        ]
        .into_iter()
        .collect()
    );
    app.assert_shared_success();
    app.click_text("Done");
    assert!(app.text().contains("Mom"));
    assert!(app.text().contains("About me"));
    assert!(app.text().contains("Request"));
    assert!(!app.text().contains("Choose passwords"));
    assert!(!app.text().contains("Share passwords"));
    assert!(app.find_text("Continue").is_none());
    app.back("Mom");
    assert!(app.text().contains("Add Contact"));
    assert!(!app.text().contains("Shared 2 times"));
}

#[test]
fn sharing_review_distinguishes_contacts_with_the_same_friendly_name() {
    let mut app = TestApp::new();
    let first = other_receiving_name();
    let second: Name = "0379be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        .parse()
        .unwrap();
    for name in [first, second] {
        Profile::from_name(&mut app.ctx, name).send(ProfileAction::ChangeUsername("Alex".into()));
    }
    app.reopen();
    app.add_login();
    app.click_text("ui@example.test");
    app.click_text("Share");
    // The contact picker identifies otherwise identical labels by Peanut Butter Name.
    app.click_text(&first.to_string());
    app.click_text(&second.to_string());
    app.click_text("Continue");
    let review = app.text();
    assert!(review.contains("Share passwords"));
    for name in [first, second] {
        assert!(review.contains(&format!("Alex ({})", short_name(name))));
        assert!(!review.contains(&name.to_string()));
    }
    assert_eq!(review.lines().filter(|line| *line == "To").count(), 1);
    assert!(!review.contains("the private UI credential"));
    assert!(app.transfers().is_empty());
    app.click_text("Send");
    let recipients = app
        .transfers()
        .into_values()
        .map(|transfer| transfer.pending().recipient)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(recipients, [first, second].into_iter().collect());
    app.assert_shared_success();
    app.click_text("Done");
    assert!(app.text().contains("Login details"));
}

#[test]
fn sharing_success_waits_until_every_transfer_is_confirmed() {
    let memory = MemoryAir::default();
    let mut app = TestApp::with_air(memory.clone());
    let mom = other_receiving_name();
    let alex: Name = "0379be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        .parse()
        .unwrap();
    Profile::from_name(&mut app.ctx, mom).send(ProfileAction::ChangeUsername("Mom".into()));
    Profile::from_name(&mut app.ctx, alex).send(ProfileAction::ChangeUsername("Alex".into()));
    app.reopen();
    app.add_login();
    app.click_text("ui@example.test");
    app.click_text("Share");
    app.click_text("Mom");
    app.click_text("Alex");
    app.click_text("Continue");
    memory.confirm_transfers(false);
    app.click_text("Send");
    assert!(app.text().contains("Sharing…"));
    assert!(!app.text().contains("Shared!"));
    assert!(!app.text().contains("the private UI credential"));
    let transfers = app.transfers();
    assert_eq!(transfers.len(), 2);
    let ids = transfers.keys().copied().collect::<Vec<_>>();
    assert!(
        transfers
            .values()
            .all(|transfer| transfer.confirmed().is_none())
    );
    memory.confirm_transfer(ids[0]);
    // MemoryAir has no asynchronous notifications, so build a fresh result page.
    app.open(flow(share_sent(app.transfers().into_values().collect())));
    assert!(app.text().contains("Sharing…"));
    assert!(!app.text().contains("Shared!"));
    memory.confirm_transfer(ids[1]);
    app.open(flow(share_sent(app.transfers().into_values().collect())));
    app.assert_shared_success();
    assert_eq!(app.transfers().len(), 2);
    assert_eq!(app.entries().len(), 1);
}

#[test]
fn saved_password_search_finds_usernames_but_not_credentials() {
    let mut app = TestApp::new();
    let mut amazon = requested_login("Amazon", "family@example.test", "private-search-needle");
    amazon.url = "https://amazon.example.test".into();
    amazon.notes = "private-note-needle".into();
    for entry in [
        amazon,
        requested_login("Netflix", "stream@example.test", "stream secret"),
    ] {
        app.model.session.borrow_mut().save(entry).unwrap();
    }
    app.reopen();
    app.header_right("Passwords");
    assert!(app.text().contains("Search passwords"));
    app.click_text("Continue");
    assert!(app.text().contains("Search passwords"));
    app.click_text("Search passwords...");
    app.type_text("private-search-needle");
    assert!(app.find_text("Amazon").is_none());
    app.set_visible_input("private-search-needle", "private-note-needle");
    assert!(app.find_text("Amazon").is_none());
    app.set_visible_input("private-note-needle", "FAMILY");
    assert!(app.find_text("Amazon").is_some());
    assert!(app.find_text("Netflix").is_none());
    app.click_text("Amazon");
    assert!(app.find_text("Amazon · family@example.test").is_some());
    assert!(!app.text().contains("private-search-needle"));
    app.click_text("Continue");
    assert!(app.text().contains("Login details"));
    assert!(app.text().contains("private-search-needle"));
    assert!(app.text().contains("private-note-needle"));
    assert!(app.transfers().is_empty());
    assert!(app.requests().is_empty());
    app.back("Login details");
    assert!(app.text().contains("Add Password"));
}

#[test]
fn peanut_butter_name_qr_uses_the_centered_native_size() {
    for mobile in [false, true] {
        let mut app = TestApp::new();
        app.ctx.window.is_mobile = mobile;
        app.ctx.window.size = if mobile {
            (390.0, 844.0)
        } else {
            (1100.0, 900.0)
        };
        app.reopen();
        app.open(flow(name_page(app.ctx.air.me())));
        app.click_text("Show QR Code");
        let header = app.header_position("Peanut Butter Name");
        let share = app.text_position("Share");
        let layout = app.layout();
        let (shape, offset) = app
            .ui
            .draw(
                &layout,
                (0.0, 0.0),
                (0.0, 0.0, app.ctx.window.size.0, app.ctx.window.size.1),
            )
            .into_iter()
            .filter_map(|instruction| match instruction.item {
                Item::Image(shape, _, _) if shape.width >= 250.0 => {
                    Some((shape, instruction.offset))
                }
                _ => None,
            })
            .max_by(|a, b| a.0.width.total_cmp(&b.0.width))
            .expect("generated QR image");
        assert!((shape.width - shape.height).abs() < 0.5);
        assert!(shape.width >= 280.0);
        assert!(shape.width <= app.ctx.window.size.0 - 32.0);
        let center = (offset.0 + shape.width / 2.0, offset.1 + shape.height / 2.0);
        assert!(
            (center.0 - share.0).abs() < 1.0,
            "QR and bumper must share a horizontal center"
        );
        assert!(
            (center.1 - (header.1 + share.1) / 2.0).abs() < 48.0,
            "QR must be centered between header and bumper"
        );
        assert!(offset.1 > header.1 && offset.1 + shape.height < share.1);
        if mobile {
            assert!((center.0 - 195.0).abs() < 1.0);
        }
    }
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
    assert!(!app.text().contains("received@example.test"));
    assert!(!app.text().contains("Received logins"));
    app.open_requests_tab();
    assert!(app.text().contains("Received logins"));
    assert!(!app.text().contains("No requests yet."));
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
    assert!(app.text().contains("Login saved"));
    assert!(!app.text().contains("Login created"));
    app.click_text("Done");
    assert!(app.text().contains("New Request"));
    assert!(!app.text().contains("Received logins"));
    assert!(!app.text().contains("received@example.test"));
    app.click_text("Passwords");
    assert!(app.text().contains("received@example.test"));
    assert!(!app.text().contains("received secret"));
}
#[test]
fn declining_never_imports_a_login() {
    let memory = MemoryAir::default();
    let id = incoming_fixture(&memory, "declined@example.test");
    let mut app = TestApp::with_air(memory);
    app.open_requests_tab();
    app.click_text("declined@example.test");
    app.click_text("Decline");
    assert!(app.entries().is_empty());
    assert_eq!(
        app.transfers()[&id].pending().status,
        TransferStatus::Declined
    );
    assert!(app.text().contains("New Request"));
    assert!(app.text().contains("No requests yet."));
    assert!(!app.text().contains("Received login"));
    assert!(!app.text().contains("declined@example.test"));
}
#[test]
fn incoming_waits_for_confirmed_private_save() {
    let memory = MemoryAir::default();
    let id = incoming_fixture(&memory, "delayed@example.test");
    let mut app = TestApp::with_air(memory.clone());
    memory.confirm_private(false);
    app.open_requests_tab();
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
    assert!(app.text().contains("Login saved"));
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
    app.click_text("New Contact");
    let recipient = other_receiving_name();
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text(recipient.to_string()));
    app.click_text("Paste Clipboard");
    app.click_text("Continue");
    let contact_name = app
        .ctx
        .air
        .instances::<Profile>()
        .list()
        .into_values()
        .find(|profile| profile.pending().name == Some(recipient))
        .unwrap()
        .pending()
        .username
        .clone();
    app.click_text("Continue");
    assert!(app.transfers().is_empty());
    app.click_text("Send");
    app.click_text("Done");
    app.click_text("Contacts");
    assert!(!app.text().contains("Shared once"));
    assert!(!app.text().contains("No logins shared"));
    assert_eq!(app.text().matches(&contact_name).count(), 1);
    app.click_text(&contact_name);
    let text = app.text();
    assert!(text.contains(&recipient.to_string()));
    assert!(text.contains("Passwords"));
    assert!(text.contains("Request"));
    assert!(!text.contains("Shared logins"));
    assert!(!text.contains("ui@example.test"));
    assert!(!text.contains("the private UI credential"));
    assert!(!text.contains("private UI notes"));
    app.click_scrolled_text("Share");
    assert!(app.text().contains("Choose passwords"));
    app.click_text("ui@example.test");
    assert!(app.text().contains("Choose passwords"));
    app.click_text("Continue");
    assert!(app.text().contains("Share passwords"));
    assert!(app.text().contains(&contact_name));
    assert!(!app.text().contains(&recipient.to_string()));
    assert_eq!(
        app.transfers().len(),
        1,
        "choosing a contact and login must not send"
    );
    app.back("Share passwords");
    app.back("Choose passwords");
    assert!(app.text().contains("Request"));
    app.click_scrolled_text("Share");
    app.click_text("ui@example.test");
    app.click_text("Continue");
    app.click_text("Send");
    assert_eq!(app.transfers().len(), 2);
    app.click_text("Contacts");
    assert!(!app.text().contains("Shared 2 times"));
    assert!(!app.text().contains("No logins shared"));
    assert_eq!(app.text().matches(&contact_name).count(), 1);
    app.click_text("Passwords");
    app.click_text("ui@example.test");
    app.click_text("Share");
    assert!(app.text().contains("New Contact"));
    assert!(app.text().contains(&contact_name));
    app.click_text(&contact_name);
    assert!(!app.text().contains("Share passwords"));
    app.click_text("Continue");
    assert!(app.text().contains("Share passwords"));
    assert_eq!(app.transfers().len(), 2);
    app.back("Share passwords");
    app.back("Share login");
    app.click_text("Share");
    app.click_text("New Contact");
    assert!(app.text().contains("Scan QR Code"));
    assert_eq!(app.transfers().len(), 2);
    // The same AIR history restores the contacts after restarting the app.
    app.model = PasswordsApp::new(&mut app.ctx);
    app.ui = app.model.clone().build_ui(&mut app.ctx, assets());
    app.flush();
    app.click_text("Contacts");
    assert!(!app.text().contains("Shared 2 times"));
    assert!(!app.text().contains("No logins shared"));
    assert_eq!(app.text().matches(&contact_name).count(), 1);
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
    app.click_text("Add Contact");
    assert!(app.text().contains("Scan QR Code"));
    assert!(app.text().contains("Enter peanut butter name..."));
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text(format!("  {name}\n")));
    app.click_text("Paste Clipboard");
    app.click_text("Continue");
    assert!(app.text().contains("About me"));
    assert!(app.text().contains("Passwords"));
    assert!(app.text().contains(&name.to_string()));
    assert!(!app.text().contains("Shared logins"));
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
    app.click_scrolled_text("Share");
    assert!(app.text().contains("Add a password to get started."));
    app.back("Choose passwords");
    app.header_right(&username);
    assert!(app.text().contains("Edit profile"));
    assert!(app.text().contains("Username"));
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
    assert_eq!(
        app.ctx
            .air
            .instances::<Profile>()
            .list()
            .into_values()
            .next()
            .unwrap()
            .pending()
            .username,
        "Alex"
    );
    app.back("Edit profile");
    // The in-memory AIR adapter has no asynchronous profile notifications.
    app.model.changes.notify();
    app.flush();
    assert!(app.text().contains("Alex"));
    app.back("Alex");
    assert!(!app.text().contains("No logins shared"));
    assert_eq!(app.text().matches("Alex").count(), 1);
    // Adding the same Peanut Butter Name again opens the saved contact and preserves its label.
    app.click_text("Add Contact");
    app.ctx
        .hardware
        .set_clipboard(&hardware::Data::Text(name.to_string()));
    app.click_text("Paste Clipboard");
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
    app.click_text("Continue");
    assert!(app.text().contains("Share passwords"));
    assert!(app.text().contains("Alex"));
    assert!(!app.text().contains(&name.to_string()));
    assert!(app.transfers().is_empty());
    app.click_text("Send");
    assert_eq!(app.transfers().len(), 1);
    app.click_text("Contacts");
    assert_eq!(app.text().matches("Alex").count(), 1);
    assert!(!app.text().contains("Shared once"));
    assert!(!app.text().contains("No logins shared"));
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
        app.click_text("Add Contact");
        app.click_text("Continue");
        assert!(app.ctx.air.instances::<Profile>().list().is_empty());
        app.ctx.hardware.set_clipboard(&hardware::Data::Text(value));
        app.click_text("Paste Clipboard");
        assert!(app.text().contains(error));
        assert!(app.text_position(error).1 > app.text_position("Add contact").1);
        app.click_text("Continue");
        assert!(app.ctx.air.instances::<Profile>().list().is_empty());
        assert!(app.transfers().is_empty());
        app.back("Add contact");
    }
    app.click_text("Add Contact");
    app.click_text("Enter peanut butter name...");
    app.type_text(&other_receiving_name().to_string());
    app.back("Add contact");
    assert!(app.ctx.air.instances::<Profile>().list().is_empty());
    assert!(
        app.text()
            .contains("Add a contact or share a login to get started.")
    );
}

fn requested_login(title: &str, username: &str, password: &str) -> Entry {
    let mut entry = Entry::new();
    entry.title = title.into();
    entry.username = username.into();
    entry.password = password.into();
    entry
}

fn pending_request(memory: &MemoryAir) -> Id {
    memory.request(LoginRequest {
        requester: other_receiving_name(),
        recipient: Name::orange_me(),
        subject: "Amazon access".into(),
        status: RequestStatus::Pending,
        created_at: 1,
        responses: vec![],
        response_saved: false,
    })
}

#[test]
fn empty_requests_centers_one_message_and_keeps_new_request_available() {
    for mobile in [false, true] {
        let mut app = TestApp::new();
        app.ctx.window.is_mobile = mobile;
        app.ctx.window.size = if mobile {
            (390.0, 844.0)
        } else {
            (1100.0, 900.0)
        };
        app.reopen();
        if mobile {
            // Mobile navigation uses icon-only tabs. Select the rightmost
            // bottom tab through its actual drawn position.
            let layout = app.layout();
            let tab = app
                .ui
                .draw(
                    &layout,
                    (0.0, 0.0),
                    (0.0, 0.0, app.ctx.window.size.0, app.ctx.window.size.1),
                )
                .into_iter()
                .filter_map(|instruction| match instruction.item {
                    Item::Svg(shape, _, _) => {
                        let point = (
                            instruction.offset.0 + shape.width / 2.0,
                            instruction.offset.1 + shape.height / 2.0,
                        );
                        (point.1 > app.ctx.window.size.1 - 80.0).then_some(point)
                    }
                    _ => None,
                })
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .expect("Requests tab icon");
            app.click_at(tab.0, tab.1);
        } else {
            app.click_text("Requests");
        }
        let text = app.text();
        assert!(
            !text
                .lines()
                .any(|line| line == "Received" || line == "Sent")
        );
        let layout = app.layout();
        let message = app
            .ui
            .draw(
                &layout,
                (0.0, 0.0),
                (0.0, 0.0, app.ctx.window.size.0, app.ctx.window.size.1),
            )
            .into_iter()
            .find_map(|instruction| match instruction.item {
                Item::Text(text)
                    if text
                        .spans
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>()
                        == "No requests yet.\nYou can make one below." =>
                {
                    assert_eq!(text.align, prism::drawable::text::Align::Center);
                    // Center-aligned text is positioned within its allocated width,
                    // which can be wider than the longest line's measured advance.
                    Some((
                        instruction.offset.0 + text.width.unwrap_or(text.size().0) / 2.0,
                        instruction.offset.1 + text.size().1 / 2.0,
                    ))
                }
                _ => None,
            })
            .expect("centered empty-state message");
        let bumper = app.text_position("New Request");
        assert!(
            (message.0 - bumper.0).abs() < 5.0,
            "mobile={mobile}, message={message:?}, bumper={bumper:?}"
        );
        assert!(message.1 > app.ctx.window.size.1 * 0.3);
        assert!(message.1 < app.ctx.window.size.1 * 0.7);
        assert!(bumper.1 > message.1);
        app.click_text("New Request");
        assert!(app.text().contains("Search contact name..."));
        assert!(app.text().contains("New Contact"));
    }
}

#[test]
fn requested_password_rows_render_the_request_illustration() {
    let memory = MemoryAir::default();
    pending_request(&memory);
    let mut app = TestApp::with_air(memory);
    app.click_text("Requests");
    let AvatarContent::Image(illustration) =
        AvatarContent::image(app.theme.brand().images["illustrations/password-request"].clone())
    else {
        unreachable!()
    };
    let row_y = app.text_position("Amazon access").1;
    let layout = app.layout();
    let rendered = app
        .ui
        .draw(&layout, (0.0, 0.0), (0.0, 0.0, 1100.0, 900.0))
        .into_iter()
        .any(|instruction| match instruction.item {
            Item::Image(shape, image, _) if image == illustration => {
                (instruction.offset.1 + shape.height / 2.0 - row_y).abs() < 45.0
            }
            _ => false,
        });
    assert!(
        rendered,
        "the request row must use the bundled illustration"
    );
}

#[test]
fn named_contact_request_validates_inline_and_requires_explicit_send() {
    let memory = MemoryAir::default();
    let mut app = TestApp::with_air(memory.clone());
    let mom = other_receiving_name();
    Profile::from_name(&mut app.ctx, mom).send(ProfileAction::ChangeUsername("Mom".into()));
    app.reopen();
    app.click_text("Contacts");
    app.click_text("Mom");
    app.click_scrolled_text("Request");
    app.click_text("Continue");
    assert!(app.requests().is_empty());
    assert!(!app.text().contains("Confirm request"));
    app.click_text("Enter app or website...");
    app.type_text(&"a".repeat(257));
    let error = "This request is too long. Use a shorter app or website name.";
    assert!(app.text().contains(error));
    assert!(app.text_position(error).1 > app.header_position("Request password").1);
    app.click_text("Continue");
    assert!(app.requests().is_empty());
    app.back("Request password");
    app.click_scrolled_text("Request");
    app.click_text("Enter app or website...");
    app.type_text("  Amazon  ");
    app.click_text("Continue");
    let confirmation = app.text();
    assert!(confirmation.contains("Confirm request"));
    assert!(confirmation.contains("Mom"));
    assert!(confirmation.contains(&mom.to_string()));
    assert!(confirmation.contains("Amazon"));
    assert!(app.requests().is_empty(), "confirmation must not send");
    app.click_text("Send Request");
    let requests = app.requests();
    assert_eq!(requests.len(), 1);
    let request = requests.into_values().next().unwrap();
    let snapshot = request.pending();
    assert_eq!(snapshot.requester, Name::orange_me());
    assert_eq!(snapshot.recipient, mom);
    assert_eq!(snapshot.subject, "Amazon");
    assert_eq!(snapshot.status, RequestStatus::Pending);
    assert!(snapshot.responses.is_empty());
    assert_eq!(memory.request_notifications(request.id()), Some(true));
    assert!(app.entries().is_empty());
    assert!(app.transfers().is_empty());
    assert!(app.text().contains("Requested 'Amazon'"));
}

#[test]
fn sent_request_has_a_minimal_dynamic_success_and_simple_saved_details() {
    let mut app = TestApp::new();
    let dad = other_receiving_name();
    Profile::from_name(&mut app.ctx, dad).send(ProfileAction::ChangeUsername("Dad".into()));
    app.reopen();
    app.click_text("Requests");
    app.click_text("New Request");
    app.click_text("Dad");
    app.click_text("Continue");
    app.click_text("Enter app or website...");
    let subject = "Dad's Password";
    app.type_text(subject);
    app.click_text("Continue");
    assert!(app.requests().is_empty());
    app.click_text("Send Request");

    let success = app.text();
    assert!(success.contains("Requested 'Dad's Password'"));
    assert_eq!(success.matches(subject).count(), 1);
    assert!(success.lines().any(|line| line == "Done"));
    for redundant in [
        "To",
        "App or website",
        "Request sent",
        "Password request",
        "Confirm request",
        "Dad",
    ] {
        assert!(
            !success.lines().any(|line| line == redundant),
            "unexpected success content: {redundant}"
        );
    }
    assert!(!success.contains(&dad.to_string()));
    let sent_art = app.theme.brand().images["illustrations/request-sent"].clone();
    let layout = app.layout();
    assert!(
        app.ui
            .draw(&layout, (0.0, 0.0), (0.0, 0.0, 1100.0, 900.0))
            .into_iter()
            .any(|instruction| matches!(instruction.item,
            Item::Image(_, image, _) if image == sent_art))
    );

    app.click_text("Done");
    assert!(app.text().contains("Sent"));
    assert!(app.text().contains("New Request"));
    app.click_text(subject);
    let details = app.text();
    for field in ["Requested", "To", "Dad", subject, "Done"] {
        assert_eq!(
            details.lines().filter(|line| *line == field).count(),
            1,
            "field {field}"
        );
    }
    assert!(!details.contains("App or website"));
    assert!(!details.contains("Request sent"));
    assert!(!details.contains(&dad.to_string()));
    let request_art = app.theme.brand().images["illustrations/password-request"].clone();
    let layout = app.layout();
    assert!(
        app.ui
            .draw(&layout, (0.0, 0.0), (0.0, 0.0, 1100.0, 900.0))
            .into_iter()
            .any(|instruction| matches!(instruction.item,
            Item::Image(_, image, _) if image == request_art))
    );
    assert!(app.entries().is_empty());
    assert_eq!(app.requests().len(), 1);
}

#[test]
fn requests_tab_starts_a_request_with_contact_selection() {
    let memory = MemoryAir::default();
    let incoming = pending_request(&memory);
    let mut app = TestApp::with_air(memory);
    let mom = other_receiving_name();
    Profile::from_name(&mut app.ctx, mom).send(ProfileAction::ChangeUsername("Mom".into()));
    app.reopen();
    assert!(!app.text().contains("Amazon access"));
    app.click_text("Requests");
    assert!(app.text().contains("Amazon access"));
    assert!(app.text().contains("New Request"));
    app.click_text("New Request");
    assert!(app.text().contains("Search contact name..."));
    assert!(app.text().contains("New Contact"));
    app.click_text("Continue");
    assert_eq!(app.requests().len(), 1, "a recipient is required");
    app.click_text("Mom");
    app.click_text("Continue");
    app.click_text("Enter app or website...");
    app.type_text("Netflix");
    app.click_text("Continue");
    assert!(app.text().contains("Confirm request"));
    assert!(app.text().contains("Mom"));
    assert!(app.text().contains("Netflix"));
    assert_eq!(app.requests().len(), 1, "review must not send");
    app.click_text("Send Request");
    let sent = app
        .requests()
        .into_iter()
        .find(|(id, _)| *id != incoming)
        .unwrap()
        .1;
    assert_eq!(sent.pending().requester, Name::orange_me());
    assert_eq!(sent.pending().recipient, mom);
    assert_eq!(sent.pending().subject, "Netflix");
    assert!(sent.pending().responses.is_empty());
    assert!(app.entries().is_empty());
    assert!(app.transfers().is_empty());
}

#[test]
fn denying_a_password_request_never_shares_or_imports_credentials() {
    let memory = MemoryAir::default();
    let id = pending_request(&memory);
    let mut app = TestApp::with_air(memory);
    assert!(!app.text().contains("Amazon access"));
    app.click_text("Requests");
    assert!(app.text().contains("New Request"));
    app.click_text("Amazon access");
    assert!(app.text().contains("Accept"));
    app.click_scrolled_text("Deny");
    let request = app.requests().remove(&id).unwrap();
    assert_eq!(request.pending().status, RequestStatus::Denied);
    assert!(request.pending().responses.is_empty());
    assert!(app.entries().is_empty());
    assert!(app.transfers().is_empty());
    assert!(app.text().contains("Request denied"));
    assert!(app.text().contains("New Request"));
    assert!(app.find_text("Done").is_none());
    assert!(app.find_text("Accept").is_none());
}

#[test]
fn accept_and_deny_are_bumpers_and_deny_returns_to_requests_on_desktop_and_mobile() {
    for mobile in [false, true] {
        let memory = MemoryAir::default();
        let id = pending_request(&memory);
        let mut app = TestApp::with_air(memory);
        app.ctx.window.is_mobile = mobile;
        app.ctx.window.size = if mobile {
            (390.0, 844.0)
        } else {
            (1100.0, 900.0)
        };
        app.reopen();
        app.open_requests_tab();
        app.click_text("Amazon access");
        let request_y = app.text_position("Amazon access").1;
        let accept_y = app.text_position("Accept").1;
        let deny_y = app.text_position("Deny").1;
        assert!(accept_y > request_y && deny_y > request_y);
        assert!(
            accept_y > app.ctx.window.size.1 * 0.6,
            "Accept must be in the bumper (mobile={mobile})"
        );
        assert!(
            deny_y > app.ctx.window.size.1 * 0.6,
            "Deny must be in the bumper (mobile={mobile})"
        );
        app.click_text("Deny");
        assert!(app.text().contains("New Request"));
        assert!(app.text().contains("Request denied"));
        assert!(app.find_text("Done").is_none());
        assert_eq!(app.requests()[&id].pending().status, RequestStatus::Denied);
        assert!(app.requests()[&id].pending().responses.is_empty());
        assert!(app.entries().is_empty());
    }
}

#[test]
fn accepting_and_selecting_multiple_logins_only_shares_after_continue() {
    let memory = MemoryAir::default();
    let id = pending_request(&memory);
    let mut app = TestApp::with_air(memory);
    let entries = [
        requested_login(
            "Amazon personal",
            "mom@example.test",
            "private personal secret",
        ),
        requested_login(
            "Amazon family",
            "family@example.test",
            "private family secret",
        ),
    ];
    for entry in &entries {
        app.model.session.borrow_mut().save(entry.clone()).unwrap();
    }
    app.reopen();
    app.click_text("Requests");
    app.click_scrolled_text("Amazon access");
    app.click_text("Accept");
    let selector = app.text();
    assert!(selector.contains("Choose passwords"));
    assert!(!selector.contains("private personal secret"));
    assert!(!selector.contains("private family secret"));
    assert!(selector.contains("Search passwords..."));
    assert!(app.find_text("Select").is_none());
    assert!(app.find_text("Selected").is_none());
    assert!(app.find_text("Continue").is_some());
    app.click_text("Continue");
    assert!(app.text().contains("Choose passwords"));
    let accepted = app.requests().remove(&id).unwrap();
    assert_eq!(accepted.pending().status, RequestStatus::Accepted);
    assert!(accepted.pending().responses.is_empty());

    app.click_scrolled_text("Amazon personal");
    assert!(app.find_text("Amazon personal").is_none());
    assert!(
        app.find_text("Amazon personal · mom@example.test")
            .is_some()
    );
    app.click_scrolled_text("Amazon personal · mom@example.test");
    assert!(app.find_text("Amazon personal").is_some());
    assert!(app.find_text("Continue").is_some());
    app.click_text("Continue");
    assert!(app.text().contains("Choose passwords"));
    assert_eq!(
        app.requests()[&id].pending().status,
        RequestStatus::Accepted
    );
    assert!(app.requests()[&id].pending().responses.is_empty());
    app.click_scrolled_text("Amazon personal");
    app.click_scrolled_text("Amazon family");
    assert!(
        app.find_text("Amazon personal · mom@example.test")
            .is_some()
    );
    assert!(
        app.find_text("Amazon family · family@example.test")
            .is_some()
    );
    assert!(app.find_text("Amazon personal").is_none());
    assert!(app.find_text("Amazon family").is_none());
    assert!(app.ctx.air.instances::<Profile>().list().is_empty());
    let accepted = app.requests().remove(&id).unwrap();
    assert_eq!(accepted.pending().status, RequestStatus::Accepted);
    assert!(
        accepted.pending().responses.is_empty(),
        "selection must stay local"
    );
    assert!(app.transfers().is_empty());
    app.click_text("Continue");
    let response = app.requests().remove(&id).unwrap();
    let response = response.pending();
    assert_eq!(response.status, RequestStatus::Responded);
    assert_eq!(response.responses.len(), 2);
    for entry in entries {
        assert!(response.responses.iter().any(|shared| {
            shared.title == entry.title
                && shared.username == entry.username
                && shared.password == entry.password
        }));
    }
    assert_eq!(app.entries().len(), 2);
    assert!(app.transfers().is_empty());
    assert!(app.text().contains("Shared!"));
    assert!(!app.text().contains("Amazon personal"));
    assert!(!app.text().contains("private personal secret"));
    assert!(app.text().contains("Done"));
    assert!(!app.text().contains("Shared passwords"));
    let illustration = app.theme.brand().images["illustrations/passwords-shared"].clone();
    let layout = app.layout();
    assert!(app.ui.draw(&layout, (0.0, 0.0), (0.0, 0.0, 1100.0, 900.0))
        .into_iter().any(|instruction| matches!(instruction.item, Item::Image(_, image, _) if image == illustration)),
        "sharing success must show its custom illustration");
}

#[test]
fn mobile_password_selection_keeps_chips_search_and_continue_reachable() {
    let memory = MemoryAir::default();
    let id = pending_request(&memory);
    let mut app = TestApp::with_air(memory);
    for entry in [
        requested_login("Amazon", "mom@example.test", "mobile personal secret"),
        requested_login("Amazon", "family@example.test", "mobile family secret"),
        requested_login("Netflix", "stream@example.test", "mobile stream secret"),
    ] {
        app.model.session.borrow_mut().save(entry).unwrap();
    }
    app.ctx.window.is_mobile = true;
    app.ctx.window.size = (390.0, 844.0);
    app.reopen();
    app.open_requests_tab();
    app.click_text("Amazon access");
    app.click_text("Accept");
    assert!(app.text().contains("Choose passwords"));
    assert!(!app.hardware.keyboard.0.load(Ordering::Relaxed));
    assert!((app.layout().0.0 - 390.0).abs() < 0.5);

    // Same-site accounts remain distinguishable by their username in each chip.
    app.click_scrolled_text("mom@example.test");
    app.click_scrolled_text("family@example.test");
    assert!(!app.hardware.keyboard.0.load(Ordering::Relaxed));
    for chip in ["Amazon · mom@example.test", "Amazon · family@example.test"] {
        let (x, y) = app.text_position(chip);
        assert!(
            (0.0..390.0).contains(&x),
            "chip outside mobile width: {chip}"
        );
        assert!(
            (0.0..844.0).contains(&y),
            "chip must remain reachable: {chip}"
        );
    }
    assert!((app.layout().0.0 - 390.0).abs() < 0.5);
    app.click_text("Amazon · mom@example.test");
    assert!(app.find_text("Amazon · mom@example.test").is_none());
    assert!(app.find_text("mom@example.test").is_some());
    assert!(app.find_text("Amazon · family@example.test").is_some());
    assert!(app.requests()[&id].pending().responses.is_empty());

    app.click_text("Search passwords...");
    app.type_text("str");
    app.type_text("eam");
    assert!(app.hardware.keyboard.0.load(Ordering::Relaxed));
    // Mobile search uses the native keyboard, without drawing another key grid.
    for key in ["q", "w", "e", "Space"] {
        assert!(app.find_text(key).is_none());
    }
    assert!(app.find_text("Netflix").is_some());
    assert!(app.find_text("mom@example.test").is_none());
    app.click_scrolled_text("Netflix");
    assert!(app.find_text("Netflix · stream@example.test").is_some());
    assert!(app.find_text("Amazon · family@example.test").is_some());
    assert!(app.find_text("stream").is_some());
    assert!((app.layout().0.0 - 390.0).abs() < 0.5);
    let picker = app.text();
    for secret in [
        "mobile personal secret",
        "mobile family secret",
        "mobile stream secret",
    ] {
        assert!(!picker.contains(secret));
    }
    let (x, y) = app.text_position("Continue");
    assert!((0.0..390.0).contains(&x));
    assert!(
        (844.0 * 0.6..844.0).contains(&y),
        "Continue must stay in the mobile bumper"
    );
    app.click_text("Continue");
    assert!(app.text().contains("Shared!"));
    assert!(!app.hardware.keyboard.0.load(Ordering::Relaxed));
    let response = app.requests().remove(&id).unwrap();
    let response = response.pending();
    assert_eq!(response.responses.len(), 2);
    let usernames = response
        .responses
        .iter()
        .map(|entry| entry.username.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        usernames,
        ["family@example.test", "stream@example.test"]
            .into_iter()
            .collect()
    );
    assert_eq!(app.entries().len(), 3);
    assert!(app.transfers().is_empty());
}

#[test]
fn request_password_search_preserves_selection_and_never_searches_credentials() {
    let memory = MemoryAir::default();
    let id = pending_request(&memory);
    let mut app = TestApp::with_air(memory);
    for entry in [
        requested_login("Amazon personal", "mom@example.test", "privateonlyneedle"),
        requested_login("Amazon family", "family@example.test", "family secret"),
        requested_login("Netflix", "stream@example.test", "stream secret"),
    ] {
        app.model.session.borrow_mut().save(entry).unwrap();
    }
    app.reopen();
    app.click_text("Requests");
    app.click_scrolled_text("Amazon access");
    app.click_text("Accept");
    app.click_text("Search passwords...");
    app.type_text("privateonlyneedle");
    let search_id = app.text_input_id("privateonlyneedle");
    assert!(app.text().contains("No matching items."));
    for title in ["Amazon personal", "Amazon family", "Netflix"] {
        assert!(app.find_text(title).is_none());
    }
    assert!(app.find_text("Continue").is_some());
    app.click_text("Continue");
    assert!(app.text().contains("Choose passwords"));
    assert!(app.requests()[&id].pending().responses.is_empty());

    app.set_visible_input("privateonlyneedle", "");
    app.click_text("Search passwords...");
    app.type_text("fam");
    // Typing continues through a results refresh without refocusing the input.
    app.type_text("ily");
    assert_eq!(app.text_input_id("family"), search_id);
    assert!(app.find_text("Amazon family").is_some());
    assert!(app.find_text("Amazon personal").is_none());
    assert!(app.find_text("Netflix").is_none());
    assert!(!app.text().contains("family secret"));
    app.click_scrolled_text("Amazon family");
    assert!(
        app.find_text("Amazon family · family@example.test")
            .is_some()
    );
    assert!(app.find_text("Amazon family").is_none());
    assert_eq!(app.text_input_id("family"), search_id);

    app.set_visible_input("family", "NETFLIX");
    assert_eq!(app.text_input_id("NETFLIX"), search_id);
    assert!(
        app.find_text("Amazon family · family@example.test")
            .is_some()
    );
    assert!(app.find_text("Netflix").is_some());
    assert!(app.find_text("Amazon personal").is_none());
    app.click_scrolled_text("Netflix");
    assert!(app.find_text("Netflix · stream@example.test").is_some());
    // Removing and selecting the same row again keeps a unique selected key.
    app.click_scrolled_text("Netflix · stream@example.test");
    assert!(app.find_text("Netflix · stream@example.test").is_none());
    assert_eq!(app.text_input_id("NETFLIX"), search_id);
    app.click_scrolled_text("Netflix");
    assert!(app.find_text("Netflix · stream@example.test").is_some());
    assert!(
        app.find_text("Amazon family · family@example.test")
            .is_some()
    );
    assert!(!app.text().contains("family secret"));
    assert!(!app.text().contains("stream secret"));
    assert!(app.requests()[&id].pending().responses.is_empty());
    app.click_text("Continue");
    assert!(app.text().contains("Shared!"));
    let response = app.requests().remove(&id).unwrap();
    let response = response.pending();
    assert_eq!(response.responses.len(), 2);
    assert!(
        response
            .responses
            .iter()
            .any(|entry| entry.title == "Amazon family")
    );
    assert!(
        response
            .responses
            .iter()
            .any(|entry| entry.title == "Netflix")
    );
    assert!(
        !response
            .responses
            .iter()
            .any(|entry| entry.title == "Amazon personal")
    );
    assert!(app.transfers().is_empty());
}

#[test]
fn sending_a_request_response_waits_for_air_confirmation() {
    let memory = MemoryAir::default();
    let id = pending_request(&memory);
    let mut app = TestApp::with_air(memory.clone());
    app.model
        .session
        .borrow_mut()
        .save(requested_login(
            "Amazon personal",
            "mom@example.test",
            "private response secret",
        ))
        .unwrap();
    app.reopen();
    app.click_text("Requests");
    app.click_scrolled_text("Amazon access");
    app.click_text("Accept");
    memory.hold_request_confirmation(id);
    app.click_scrolled_text("Amazon personal");
    app.click_text("Continue");
    let text = app.text();
    assert!(text.contains("Sending logins…"));
    assert!(!text.contains("Shared!"));
    assert!(!text.contains("private response secret"));
    let pending = app.requests().remove(&id).unwrap();
    assert_eq!(pending.pending().status, RequestStatus::Responded);
    assert_eq!(pending.confirmed().unwrap().status, RequestStatus::Accepted);
    assert!(pending.confirmed().unwrap().responses.is_empty());

    app.reopen();
    app.click_text("Requests");
    assert!(app.text().contains("Amazon personal"));
    assert!(app.text().contains("Sending logins…"));
    assert!(!app.text().contains("No requests yet."));

    memory.confirm_request(id);
    // MemoryAir deliberately has no asynchronous notifications: reopen the
    // requests tab to deliver the now-confirmed request snapshot.
    app.reopen();
    app.click_text("Requests");
    assert!(app.text().contains("No requests yet."));
    assert!(!app.text().contains("Amazon personal"));
    assert!(!app.text().contains("Amazon access"));
    assert!(!app.text().contains("Sending logins…"));
    assert_eq!(app.requests()[&id].confirmed().unwrap().responses.len(), 1);
    assert_eq!(app.entries().len(), 1);
    assert!(app.transfers().is_empty());
}

#[test]
fn accepted_unfinished_request_stays_available_after_reopening() {
    let memory = MemoryAir::default();
    let id = pending_request(&memory);
    let mut app = TestApp::with_air(memory);
    app.click_text("Requests");
    app.click_text("Amazon access");
    app.click_text("Accept");
    assert!(app.text().contains("Choose passwords"));
    assert_eq!(
        app.requests()[&id].pending().status,
        RequestStatus::Accepted
    );
    assert!(app.requests()[&id].pending().responses.is_empty());

    app.reopen();
    app.click_text("Requests");
    assert!(app.text().contains("Amazon access"));
    assert!(!app.text().contains("No requests yet."));
    app.click_text("Amazon access");
    app.click_text("Accept");
    assert!(app.text().contains("Choose passwords"));
    assert!(app.requests()[&id].pending().responses.is_empty());
    assert!(app.entries().is_empty());
}

#[test]
fn request_summary_deduplicates_services_without_merging_shared_accounts() {
    let memory = MemoryAir::default();
    let responses = vec![
        requested_login(
            "Amazon",
            "personal@example.test",
            "personal returned secret",
        ),
        requested_login("Amazon", "family@example.test", "family returned secret"),
    ];
    let id = memory.request(LoginRequest {
        requester: Name::orange_me(),
        recipient: other_receiving_name(),
        subject: "Dad's requested access".into(),
        status: RequestStatus::Responded,
        created_at: 1,
        responses: responses.clone(),
        response_saved: false,
    });
    let mut app = TestApp::with_air(memory);
    app.open_requests_tab();
    let summary = app.text();
    assert_eq!(summary.lines().filter(|line| *line == "Amazon").count(), 1);
    assert!(!summary.contains("Amazon, Amazon"));
    assert!(!summary.contains("Dad's requested access"));
    assert!(!summary.contains("returned secret"));
    app.click_text("Amazon");
    let review = app.text();
    assert_eq!(review.lines().filter(|line| *line == "Amazon").count(), 2);
    assert!(review.contains("personal@example.test"));
    assert!(review.contains("family@example.test"));
    assert!(!review.contains("App or website"));
    assert!(!review.contains("returned secret"));
    for entry in &responses {
        app.click_text(&entry.username);
        let detail = app.text();
        assert!(detail.contains("Shared login"));
        assert!(detail.contains(&entry.password));
        assert_eq!(detail.lines().filter(|line| *line == "Amazon").count(), 1);
        for other in responses
            .iter()
            .filter(|other| other.username != entry.username)
        {
            assert!(!detail.contains(&other.password));
        }
        assert!(app.entries().is_empty());
        app.click_text("Done");
        assert!(app.text().contains("Save all"));
        assert!(!app.text().contains("returned secret"));
    }
    app.click_text("Save all");
    let saved = app.entries();
    assert_eq!(saved.len(), 2);
    for entry in &responses {
        assert!(saved.iter().any(|saved| saved.title == entry.title
            && saved.username == entry.username
            && saved.password == entry.password));
    }
    assert!(app.requests()[&id].pending().response_saved);
    app.click_text("Done");
    assert!(app.text().contains("No requests yet."));
    assert!(app.find_text("Amazon").is_none());
}

#[test]
fn request_response_needs_save_all_and_does_not_duplicate_after_reopening() {
    let memory = MemoryAir::default();
    let response = vec![
        requested_login(
            "Amazon personal",
            "mom@example.test",
            "received personal secret",
        ),
        requested_login(
            "Amazon family",
            "family@example.test",
            "received family secret",
        ),
    ];
    let id = memory.request(LoginRequest {
        requester: Name::orange_me(),
        recipient: other_receiving_name(),
        subject: "Amazon access".into(),
        status: RequestStatus::Responded,
        created_at: 1,
        responses: response.clone(),
        response_saved: false,
    });
    let mut app = TestApp::with_air(memory.clone());
    assert!(!app.text().contains("Amazon access"));
    app.click_text("Requests");
    assert!(app.text().contains("Logins ready to save"));
    assert!(!app.text().contains("received personal secret"));
    assert!(!app.text().contains("received family secret"));
    assert!(app.entries().is_empty());
    assert!(app.text().contains("Amazon personal, Amazon family"));
    assert!(!app.text().contains("Amazon access"));
    app.click_text("Amazon personal, Amazon family");
    app.ctx.window.size = (1100.0, 1800.0);
    let review = app.text();
    assert!(
        !review.contains("Amazon access"),
        "the request's original label is not a returned login"
    );
    assert_eq!(review.matches("App or website").count(), 0);
    assert_eq!(review.matches("Amazon personal").count(), 1);
    assert_eq!(review.matches("Amazon family").count(), 1);
    assert!(review.contains("mom@example.test"));
    assert!(review.contains("family@example.test"));
    assert!(!review.contains("received personal secret"));
    assert!(!review.contains("received family secret"));
    assert!(app.entries().is_empty(), "review must not import");
    assert!(!app.requests()[&id].pending().response_saved);

    for entry in &response {
        app.click_text(&entry.title);
        let detail = app.text();
        assert!(detail.contains("Shared login"));
        assert_eq!(detail.matches("App or website").count(), 1);
        assert!(detail.contains(&entry.title));
        assert!(detail.contains(&entry.username));
        assert!(detail.contains(&entry.password));
        assert!(!detail.contains("Amazon access"));
        for other in response.iter().filter(|other| other.title != entry.title) {
            assert!(!detail.contains(&other.password));
        }
        assert!(app.find_text("Save all").is_none());
        assert!(
            app.entries().is_empty(),
            "opening a shared login must not save it"
        );
        app.click_text("Done");
        assert!(app.text().contains("Save all"));
        assert!(!app.text().contains(&entry.password));
        assert!(!app.requests()[&id].pending().response_saved);
    }
    app.click_text("Save all");
    assert!(app.text().contains("Logins saved"));
    assert!(app.requests()[&id].pending().response_saved);
    let saved = app.entries();
    assert_eq!(saved.len(), 2);
    for entry in &response {
        assert!(saved.iter().any(|saved| {
            saved.title == entry.title
                && saved.username == entry.username
                && saved.password == entry.password
        }));
    }
    app.click_text("Done");
    assert!(!app.text().contains("Logins ready to save"));
    assert!(app.text().contains("No requests yet."));
    assert!(!app.text().contains("Amazon personal"));
    assert!(!app.text().contains("Amazon family"));
    assert!(app.requests()[&id].confirmed().unwrap().response_saved);
    drop(app);

    let mut reopened = TestApp::with_air(memory);
    assert_eq!(reopened.entries(), saved);
    assert!(!reopened.text().contains("Amazon access"));
    assert!(!reopened.text().contains("received personal secret"));
    assert!(!reopened.text().contains("received family secret"));
    assert!(reopened.requests()[&id].pending().response_saved);
    reopened.click_text("Requests");
    assert!(reopened.text().contains("No requests yet."));
    assert!(!reopened.text().contains("Amazon personal"));
    assert!(!reopened.text().contains("Amazon family"));
}
