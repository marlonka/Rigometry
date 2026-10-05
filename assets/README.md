# Artwork

The application mark is an original monitor and pulse drawing in black and white, licensed under [MIT](../LICENSE). Navigation icons are original 24-unit vector drawings with matching rounded strokes.

`rigometry.svg` is the editable source. On Windows, run:

```powershell
pwsh -NoProfile -File scripts/generate-icon.ps1
```

This produces a 512 px PNG and a seven-resolution ICO (16–256 px). The window uses the PNG; the Windows SDK resource compiler embeds the ICO in the executable. Set `RC` only when a custom compiler path is needed.

The sidebar draws matching geometry in `src/branding.rs`. Update both representations together. Inspect the 16, 24, 32 and 48 px sizes and enlarged UI scale after changes. Grayscale edge pixels provide antialiasing, not an additional brand color.