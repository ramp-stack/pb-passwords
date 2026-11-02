use chk::{Assets, Context};
use ramp::prism::{Application, drawable::Drawable};

mod branding;
mod contacts;
mod links;
mod notifications;
mod passphrase;
mod requests;
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
        ctx.air.start::<notifications::RequestNotifications>(());
        ui::PasswordsApp::new(ctx).build_ui(
            ctx,
            Assets::new(vec![include_dir::include_dir!(
                "$CARGO_MANIFEST_DIR/resources"
            )]),
        )
    }
}
