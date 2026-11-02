# Peanut Butter Passwords visual assets

The visual direction follows the flat illustrated list icons in Fig Files:
solid color blocks, rounded silhouettes, a few simple detail strokes, and
small dark circular badges. The app mark is a peanut-butter jar with a vault
door on its label. Saved logins have one peanut illustration as their fallback.
All artwork is original; no Fig Files assets were copied. No faces, key motifs,
glossy 3D renders, or photorealistic food textures are bundled.

Palette: charcoal #151412, espresso #23211D, butter #E9BC7B, warm white #F6ECDD.

## Asset map

- `assets/app-icon.svg` → `icon.png`: flat peanut-butter jar vault app icon, 1024px.
- `resources/brand`: native logo, app-icon, error, and illustrated wordmark SVGs. The wordmark uses an organic peanut-butter smear,
  uneven knife edges and spread ridges, with dark outlined lettering for contrast.
- `resources/login-icons/peanut`: the single default login badge, SVG and 256px PNG.
- `resources/illustrations/peanut-vault`: jar-vault empty-state artwork, SVG and PNG.
- `resources/illustrations/generator`: toast with peanut-butter swirl, SVG and PNG.
- `resources/illustrations/login-saved`: jar vault and checkmark, SVG and PNG.
- `resources/illustrations/password-request`: flat envelope and peanut seal badge for request rows, SVG and 512px PNG.
- `resources/illustrations/request-sent`: peanut-seal envelope with a confirmation badge for sent-request success pages, SVG and 512px PNG.
- `resources/illustrations/passwords-shared`: two peanut-stamped login cards and a passing arrow for completed password sharing, SVG and 512px PNG.
- `ios/PeanutButterPasswordsLaunchMark.png`: transparent flat launch mark.
- `resources/icons`: custom rounded action/navigation SVGs.
  Profile and edit use lighter strokes. `check` is the filled version of the
  `unchecked` rounded square; `checkmark` keeps a separate tick for confirmations.

## Regenerate

Run `python3 assets/generate-icons.py` from any directory. It writes the native
SVG sources and exports PNGs with `resvg` (required on PATH). Regenerate the
outlined typography with:

```sh
swift -module-cache-path /private/tmp/pb-passwords-swift-cache assets/generate-wordmark.swift
```

All final artwork is code-native SVG, rasterized for the app's image avatar
API. The earlier built-in imagegen 3D explorations were rejected and replaced;
they are not bundled or referenced. No image-generation prompt is needed for
the final vector artwork.

Saved logins use a logo from the identified website or its fixed official brand-image
address when available. The peanut badge is the only fallback. Requests contain only
the public image address, never login credentials, full labels, or the full saved link.
