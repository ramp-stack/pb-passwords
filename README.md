# Peanut Butter Passwords

A small native password manager built with CHK and AIR. It opens directly to
a list of saved logins. Add/Edit Login includes an **App or website** field
for the login’s name. Saved names appear in the list and login picker;
usernames appear beneath distinct login names. Leaving the name empty
uses the username or link as a fallback. Each row opens the name, username,
password, optional notes, and link. The top-right edit icon opens an editor for every login field, with
always-visible text fields and a back arrow. The main page has an Add Password bumper action.
Add/Edit Login places Generate, Paste, and the generator checkboxes directly beneath
the Password field, before Link and Notes, with a Save bumper action.
Login details has a Share bumper action. The Passwords header opens Search passwords,
using CHK's searchable login selector to find and open a saved login.
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

The Contacts page has an **Add Contact** bumper. Paste or scan a Peanut Butter
Name to save someone before sharing anything, using the same CHK profile storage
as Orange. Contact pages use Orange’s avatar and native profile editor. A
**Passwords** data item provides **Share** and **Request** actions. Adding the
same name again opens the existing contact without duplicating it.

Contacts also includes people you have sent logins to, most recent first, using
the existing AIR sharing history. Saved contacts and repeated shares are grouped
by Peanut Butter Name. Sharing counts are not displayed.
Contact pages keep the profile and password actions together without a sharing
history list; request status is available in the Requests tab.

Share in login details uses Orange’s new-message CHK flow: search contacts,
select one or more, then Continue. **New Contact** opens the same paste/scan form
and returns to the search page with the added contact selected. Starting from a
contact instead opens the same searchable login selector as request responses,
with removable selections and support for multiple logins. Continue opens a
simple review with contact names and login list items. **Send** shares the selected
copies; **Shared!**, the custom sharing illustration, and **Done** appear only
after AIR confirms all of them. Done returns to the original contact or login.
These pages use the same CHK search, selected-contact chips, form navigation,
Paste, and Scan QR code actions as Orange. The Contacts header’s QR button shows one Peanut Butter Name
data item with Copy and Show QR code actions. The QR view’s Share bumper opens
the native share window with only that public name. These AIR names use the
same format as Orange names, but use the Peanut Butter Name shown inside this
app; each app may have a different identity.

Only a dedicated immutable copy of the selected login’s name, username, password,
link, and notes is shared. The complete private password list is never shared.
Incoming copies appear in Requests under Received logins; review and
save or decline them. The main Passwords page contains only saved logins.
Edits to a shared copy do not change the sender’s login.

The sharing screen waits for confirmation instead of reporting a queued copy as shared.
Local import uses a deterministic ID to make retries safe and preserve edits
or deletion. An acceptance is acknowledged after the private import confirms.

## Password requests

The third tab, **Requests**, lists requests made to you and has a **New Request**
bumper. Choose a contact, enter an app or website such as Amazon, then review and
press **Send Request**. You can also start from a contact’s **Request** action.
After confirmation, the sent screen shows a success illustration,
**Requested '…'**, and **Done**. Reopening an unanswered sent request shows a small
illustration and two data items: **Requested** and **To**. Denied requests keep
their denied heading.

Open an incoming request to use the **Accept** and **Deny** bumper buttons.
Deny returns to Requests, where its row shows **Request denied**.
Accepting opens **Choose passwords**, using the same generic CHK selector as
Orange’s contacts: tap a login list item to select it, or its selected chip to
remove it. Inline search matches login names and usernames; passwords and notes
are never searched. Selections remain when changing the search, and **Continue**
stays disabled until at least one login is selected. Accepting and selecting share nothing:
press **Continue** to send only the selected copies together. The simple
**Shared!** success page and custom sharing illustration appear after AIR confirms
the response. The completed incoming request then leaves the list; an unfinished
accepted request remains available to resume.

Your outgoing requests appear under **Sent** in the Requests tab. Open a response,
review its login list items, and press **Save all** to import them. Each row opens
that login’s details, keeping passwords out of the list. Retrying or reopening
never creates duplicate copies. After the save is confirmed, the request leaves
the Requests list while the imported logins remain saved. Received replies show
the actual login names, without repeating the original request label. Request rows
use a flat peanut-seal envelope illustration. With no received or sent requests,
the tab shows one centered message and its New Request action.
People you answer also become available in the
contacts picker for future sharing.

## Request notifications

Active password requests use AIR notification subscriptions independently of the
visible page. Subscriptions restore at startup and stop when requests are denied
or their responses have been saved. Push text is generic; passwords, usernames,
and requested service names are never included in the notification payload.

Cargo Piston notifications are enabled for this app, with an iOS development
registration configured for its own `org.ramp.pickle-passwords` APNs topic.
Production/TestFlight and other platform builds need their respective notification
registrations. Notifications also require the recipient’s OS permission. Plain
Cargo builds without a notification application ID continue to run without remote push.

The current Ramp release subscribes only to contracts the device already knows.
It can notify about updates to an existing request, but a brand-new request cannot
wake a closed app until inbox-level notification support is available. Opening
the app still loads new requests through AIR. Native notification delivery for
this app has not yet been verified on a device.

## Assets

The dark identity uses CHK’s black surfaces with toasted peanut and butter accents,
a flat illustrated peanut-butter jar vault app icon, and a peanut-butter smear
wordmark with dark lettering and flat knife-spread ridges.
Saved logins use the website’s logo when their link or recognizable name identifies
a public website, including personalized labels such as “Ella’s Gmail”. Logos are
fetched from the site or a fixed official brand-image address, with a session cache;
passwords, email usernames, full labels, and full saved URLs are never sent.
Larger touch icons are preferred; small icons are centered without enlargement.
One peanut illustration is the fallback when a logo is unavailable. Flat illustrations
appear in the empty state and success screen; login artwork stays on list rows. Custom rounded icons
keep actions recognizable and labels literal. Checked boxes use the filled version
of the same rounded square; contact and edit icons use lighter outlines.
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
to this project, with approved CHK additions: `Action::set_text(id, value)`
lets Generate fill a standard text field without changing the clipboard, and shared
button formatting capitalizes the first word and words longer than three letters.
The generic CHK Selector is shared with Orange for contact and login selection.
Headings, user content, and field labels retain their original casing.
Orange-specific screens, private files, GIF configuration,
legacy Xcode project, unused vendor code, unrelated permissions, and
App Store metadata were removed. The existing parent Cargo configuration
provides local CHK and its internal dependency overrides. The CHK revision in
the manifest predates `Action::set_text`; standalone builds require a CHK version
containing that addition or the existing local override.
