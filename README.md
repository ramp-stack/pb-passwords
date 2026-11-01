# Peanut Butter Passwords

A small native password manager built with CHK and AIR. It opens directly to
a list of saved logins. Add/Edit Login includes an **App or website** field
for the login’s name. Saved names appear in the list, login picker, and sharing
history; usernames appear beneath distinct login names. Leaving the name empty
uses the username or link as a fallback. Each row opens the name, username,
password, optional notes, and link. The top-right edit icon opens an editor for every login field, with
always-visible text fields and a back arrow. The main page has an Add password bumper action.
Add/Edit Login contains the generator options, Generate beside Paste,
and a Save bumper action. Login details has a Share bumper action.
Save stays disabled until the password is filled in and the form is valid.
Field errors appear beside their inputs. Creating a login shows a success page
with a secondary Done button.
All screens use CHK pages, flows, list items, and form inputs. The editor uses
CHK’s edit form with inline validation and its native Save action. CHK is the
app’s only direct UI dependency; Pelican UI and PTSD are internal dependencies
of CHK. The runtime adapter handles clipboard expiry without composing UI. There is no custom password input, visibility toggle,
master password, welcome screen, or account setup.

The generator creates three random words joined by hyphens, such as
`rainy-october-simply`. Independent options add a capital letter, a special
character, and a number. Generate fills the password field in Add/Edit Login,
preserving the other draft fields. Changing options does not replace a typed
password until Generate is pressed, and storage changes only when Save is
pressed. Passwords stay visible while editing, viewing login details, and reviewing a received login.
The bundled dictionary and its source notes are documented in
[assets/wordlist-notes.md](assets/wordlist-notes.md).

## Run

```sh
cargo piston dev macos --locked
```

For an iPhone or simulator, use `cargo piston dev ios --locked` or
`cargo piston dev ios --simulator --locked`. Apple development requires Xcode
and a signing identity. The app identifier is `org.ramp.pickle-passwords`.
Development commands do not publish the app.

## Storage

Password entries use AIR directly, following Orange's storage pattern. AIR
handles persistence and synchronization under the app's identity. The password
contract restricts updates to its owner and checks revisions to preserve edits.
There is no separate vault key, Keychain integration, or unlock step.
The bundle and AIR contract identifiers retain their previous values for
continuity. On macOS/iOS, existing installations alias the renamed application
support folder to the original folder before AIR starts, preserving saved data
and identity without copying or replacing a live database. The source folder
is `pb-passwords`; the app and package are Peanut Butter Passwords.
The iOS home-screen display name is **PB Passwords**.

Copy actions expire after 30 seconds while the app is running, without
clearing newer clipboard content.
Mobile input uses only the native keyboard, which dismisses when an input page closes.

## Sharing

The Contacts page has an **Add contact** bumper. Paste or scan a Peanut Butter
Name to save someone before sharing anything, using the same CHK profile storage
as Orange. The edit icon lets you change the contact’s display name. Adding the
same name again opens the existing contact without duplicating it.

Contacts also includes people you have sent logins to, most recent first, using
the existing AIR sharing history. Saved contacts and repeated shares are grouped
by Peanut Butter Name.
Open a contact to see which logins you shared and choose **Share a login** to send
another. Login names, usernames, and delivery status appear in contact history.

Use Share in login details to choose a previous contact, or **Someone new** to
paste or scan their Peanut Butter Name. Review the full name and press Send.
If you have no saved or previous contacts, Share opens the paste/scan form directly. Paste and Scan QR code use the same CHK
actions as Orange. The home header’s QR button shows one Peanut Butter Name
data item with Copy and Show QR code actions. The QR view’s Share bumper opens
the native share window with only that public name. These AIR names use the
same format as Orange names, but use the Peanut Butter Name shown inside this
app; each app may have a different identity.

Only a dedicated immutable copy of the selected login’s name, username, password,
link, and notes is shared. The complete private password list is never shared.
Incoming copies appear on the home page under Received logins; review and
save or decline them. Edits to a shared copy do not change the sender’s login.

Sharing status distinguishes a queued copy from one saved by the recipient.
Local import uses a deterministic ID to make retries safe and preserve edits
or deletion. An acceptance is acknowledged after the private import confirms.

## Assets

The dark identity uses CHK’s black surfaces with toasted peanut and butter accents,
a flat illustrated peanut-butter jar vault app icon, and a flat illustrated peanut-butter label wordmark.
Saved logins use the website’s logo when their link or recognizable name identifies
a public website, including personalized labels such as “Ella’s Gmail”. Logos are
fetched from the site or a fixed official brand-image address, with a session cache;
passwords, email usernames, full labels, and full saved URLs are never sent.
Larger touch icons are preferred; small icons are centered without enlargement.
One peanut illustration is the fallback when a logo is unavailable. Flat illustrations
appear in the empty state and success screen; login artwork stays on list rows. Custom rounded icons
keep actions recognizable and labels literal.
`icon.png` supplies the platform icon; Cargo Piston generates the needed sizes.
Project-specific SVG branding is in `resources/brand`, and iOS launch resources
are in `ios`. Asset sources and regeneration instructions are in
[assets/README.md](assets/README.md).

## Development

```sh
cargo fmt --check
cargo check --locked --all-targets
cargo test --locked --lib
cargo build --locked --bin peanut-butter-passwords
```

This project began as a minimal duplication of Orange. App changes are confined
to this project, with one approved CHK addition: `Action::set_text(id, value)`
lets Generate fill a standard text field without changing the clipboard. Orange-specific screens, private files, GIF configuration,
legacy Xcode project, unused vendor code, notifications, permissions, and
App Store metadata were removed. The existing parent Cargo configuration
provides local CHK and its internal dependency overrides. The CHK revision in
the manifest predates `Action::set_text`; standalone builds require a CHK version
containing that addition or the existing local override.
