use chk::{Assets, Context};
use ramp::prism::{Application, drawable::Drawable};

mod branding;
mod contacts;
mod links;
mod passphrase;
mod runtime;
mod secret;
mod sharing;
mod site_logos;
mod storage;
mod ui;

ramp::run!(PeanutButterPasswords);

pub struct PeanutButterPasswords;

impl Application for PeanutButterPasswords {
    fn name() -> String {
        crate::branding::application_name()
    }

    fn new(ctx: &mut Context) -> impl Drawable {
        ui::PasswordsApp::new(ctx).build_ui(
            ctx,
            Assets::new(vec![include_dir::include_dir!(
                "$CARGO_MANIFEST_DIR/resources"
            )]),
        )
    }
}
