"""Generate Peanut Butter Passwords' flat, code-native visual assets."""
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parent.parent
folder = root / 'resources/login-icons'
folder.mkdir(exist_ok=True)

# A sturdy pantry jar with a safe-door label. Flat shapes and a generous
# silhouette keep the jar and vault readable at both dock and QR-logo sizes.
jar = '<path d="M65 74h126v19c0 10 15 13 15 33v67c0 17-13 29-30 29H80c-17 0-30-12-30-29v-67c0-20 15-23 15-33Z" fill="#C98946"/><path d="M51 187h154v6c0 17-13 29-29 29H80c-17 0-29-12-29-29Z" fill="#A86632"/><rect x="49" y="39" width="158" height="43" rx="13" fill="#F4E4C5"/><path d="M69 51v18m24-18v18m24-18v18m24-18v18m24-18v18m24-18v18" fill="none" stroke="#B78B59" stroke-width="5" stroke-linecap="round"/><rect x="61" y="111" width="134" height="78" rx="17" fill="#F4E4C5"/><rect x="73" y="123" width="110" height="54" rx="10" fill="none" stroke="#52351F" stroke-width="5"/><path d="M73 133v8m0 18v8" fill="none" stroke="#52351F" stroke-width="10" stroke-linecap="round"/><circle cx="137" cy="150" r="21" fill="#52351F"/><path d="M137 150v-12m0 12 10 6m-10-6-10 6" fill="none" stroke="#F4E4C5" stroke-width="5" stroke-linecap="round"/><circle cx="137" cy="150" r="5" fill="#F4E4C5"/>'

# One shelled peanut, without a face, key, lighting effect, or 3D extrusion.
peanut_shell = 'M128 35c-27 0-46 20-42 47 2 19 17 27 17 46 0 18-16 29-17 49-2 26 16 44 42 44s44-18 42-44c-1-20-17-31-17-49 0-19 15-27 17-46 4-27-15-47-42-47Z'
peanut = f'<g transform="rotate(-28 128 128)"><defs><clipPath id="peanut-shell"><path d="{peanut_shell}"/></clipPath></defs><path d="{peanut_shell}" fill="#E9BC7B"/><g clip-path="url(#peanut-shell)"><path d="m105 55 48 43m-56-24 48 44m-34-69 45 40m-58 55 57 51m-61-25 49 43m-35-74 51 45m-49-81 41-44m-44 65 49-53m-49 98 45-45m-52 68 57-57m-46 73 43-43" fill="none" stroke="#B97C3D" stroke-width="5" stroke-linecap="round"/><path d="M103 65c-7 9-8 20-2 30m3 61c-6 8-9 18-6 28" fill="none" stroke="#F7DCAB" stroke-width="5" stroke-linecap="round"/></g></g>'

def illustration(art, background=""):
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">{background}{art}</svg>'

# Transparent brand mark for the app header and the standard CHK QR component.
mark = illustration(jar)
for name in ['logo', 'app_icon', 'error']:
    (root / f'resources/brand/{name}.svg').write_text(mark)

# Exactly one fallback image; website logos still take priority.
badge = illustration(peanut, '<circle cx="128" cy="128" r="124" fill="#2E261D"/>')
(folder / 'peanut.svg').write_text(badge)

# Familiar rounded controls, with a conventional padlock for password fields.
icons={
 'profile':'<circle cx="24" cy="15" r="8"/><path d="M9 41v-4c0-8 6-13 15-13s15 5 15 13v4Z"/>',
 'group':'<circle cx="18" cy="15" r="7"/><path d="M5 40v-4c0-7 5-12 13-12s13 5 13 12v4ZM32 8a7 7 0 0 1 0 14m3 5c6 1 9 5 9 11v2"/>',
 'send':'<path d="m6 8 36 16-36 16 7-16Z"/><path d="M14 24h25"/>',
 'qr_code':'<rect x="6" y="6" width="12" height="12" rx="2"/><rect x="30" y="6" width="12" height="12" rx="2"/><rect x="6" y="30" width="12" height="12" rx="2"/><path d="M30 30h5v5h7v7H30v-6M24 7v11M7 24h11m6 6v12m18-18h-8m-10 0h1"/>',
 'credential':'<path d="M15 21v-7a9 9 0 0 1 18 0v7"/><rect x="9" y="21" width="30" height="22" rx="6"/><circle cx="24" cy="31" r="2"/><path d="M24 33v4"/>',
 'infinite':'<path d="m10 35 15-15m-10-7 7-7 20 20-7 7Z"/><path d="m7 30 11 11M29 8l2-4m9 11 4-2M8 14v6m-3-3h6"/>',
 'username':'<circle cx="24" cy="15" r="8"/><path d="M9 41v-4c0-8 6-13 15-13s15 5 15 13v4Z"/>',
 'notes':'<path d="M12 6h19l8 8v27H12Z"/><path d="M30 6v10h9M19 24h13m-13 8h10"/>',
 'copy':'<rect x="17" y="16" width="24" height="26" rx="6"/><path d="M10 32H9a4 4 0 0 1-4-4V10a5 5 0 0 1 5-5h18a5 5 0 0 1 5 5"/>',
 'add':'<path d="M24 10v28M10 24h28"/>',
 'edit':'<path d="m9 31 22-22q4-4 8 0t0 8L17 39l-10 2Z"/><path d="m27 13 8 8"/>',
 'check':'<path d="m9 25 10 10L39 13"/>',
 'unchecked':'<rect x="8" y="8" width="32" height="32" rx="11"/>',
 'link':'<path d="m19 29 10-10M17 33l-3 3a8 8 0 0 1-11-11l9-9a8 8 0 0 1 11 0m2 16a8 8 0 0 0 11 0l9-9a8 8 0 0 0-11-11l-3 3"/>',
 'delete':'<path d="M7 13h34M18 12V6h12v6M12 13l2 26q0 3 4 3h12q4 0 4-3l2-26M21 21v12m7-12v12"/>',
}
for name,art in icons.items(): (root/f'resources/icons/{name}.svg').write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48"><g fill="none" stroke="#F6ECDD" stroke-width="3.8" stroke-linecap="round" stroke-linejoin="round">{art}</g></svg>')

# The empty and saved states share the jar-vault rather than a separate mascot.
sparkles = '<path d="M218 85v14m-7-7h14M32 172v10m-5-5h10" fill="none" stroke="#F4E4C5" stroke-width="5" stroke-linecap="round"/>'
empty_art = illustration('<g transform="translate(7 6) rotate(-5 128 128) scale(.95)">'+jar+'</g>'+sparkles)
saved_art = illustration('<g transform="translate(4 4) scale(.9)">'+jar+'</g><circle cx="194" cy="195" r="42" fill="#151412"/><circle cx="194" cy="195" r="32" fill="#F4E4C5"/><path d="m178 195 11 11 22-24" fill="none" stroke="#52351F" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"/>')
# A simple toast illustration keeps the generator's food reference easy to read.
toast = '<path d="M60 104c-20-17-16-43 8-51 13-5 25-2 34 2 16-10 36-10 52 0 9-4 21-7 34-2 24 8 28 34 8 51v85c0 16-11 26-27 26H87c-16 0-27-10-27-26Z" fill="#B97C3D"/><path d="M73 104c-24-20-11-44 16-35l14 6 11-7c9-6 19-6 28 0l11 7 14-6c27-9 40 15 16 35v84c0 8-5 13-13 13H86c-8 0-13-5-13-13Z" fill="#F4E4C5"/><path d="M99 97c19-13 47-9 59 4 12 12-3 26 0 41 2 16 18 29 1 40-13 8-29-2-44 2-18 4-30-4-26-20 4-15 0-24-3-37-3-13 4-22 13-30Z" fill="#C98946"/><path d="M107 115c14-10 36-9 37-1 1 10-39 15-38 28 0 10 21 8 35 16" fill="none" stroke="#F4D29B" stroke-width="7" stroke-linecap="round"/>'
generator_art = illustration('<g transform="rotate(-8 128 128)">'+toast+'</g>'+sparkles)
for name, art in [('peanut-vault', empty_art), ('generator', generator_art), ('login-saved', saved_art)]:
    (root / f'resources/illustrations/{name}.svg').write_text(art)

# The platform icon is fully opaque; iOS applies its own corner mask.
app_art = illustration('<g transform="rotate(-6 128 128)">'+jar+'</g>', '<rect width="256" height="256" fill="#151412"/>')
(root / 'assets/app-icon.svg').write_text(app_art)

renderer = shutil.which('resvg')
if not renderer:
    raise SystemExit('SVG sources saved. Install resvg to regenerate the PNG exports.')
for source in folder.glob('*.svg'):
    subprocess.run([renderer, str(source), str(source.with_suffix('.png'))], check=True)
for source in (root / 'resources/illustrations').glob('*.svg'):
    subprocess.run([renderer, '--width', '512', str(source), str(source.with_suffix('.png'))], check=True)
subprocess.run([renderer, '--width', '1024', str(root / 'assets/app-icon.svg'), str(root / 'icon.png')], check=True)
shutil.copyfile(root / 'resources/illustrations/peanut-vault.png', root / 'ios/PeanutButterPasswordsLaunchMark.png')
