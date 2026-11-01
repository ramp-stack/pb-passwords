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
- `resources/brand`: native logo, app-icon, error, and illustrated wordmark SVGs. The wordmark combines a flat spread-shaped label,
  butter-colored outlined lettering, and a small peanut detail.
- `resources/login-icons/peanut`: the single default login badge, SVG and 256px PNG.
- `resources/illustrations/peanut-vault`: jar-vault empty-state artwork, SVG and PNG.
- `resources/illustrations/generator`: toast with peanut-butter swirl, SVG and PNG.
- `resources/illustrations/login-saved`: jar vault and checkmark, SVG and PNG.
- `ios/PeanutButterPasswordsLaunchMark.png`: transparent flat launch mark.
- `resources/icons`: custom rounded action/navigation SVGs.

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
