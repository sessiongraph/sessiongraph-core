# Icons

Place the application icons here before building installers. Required files
(as referenced by `tauri.conf.json`):

- `32x32.png`
- `128x128.png`
- `128x128@2x.png`
- `icon.icns` (macOS)
- `icon.ico` (Windows)

Generate them from a single source PNG with:

```bash
pnpm tauri icon path/to/source.png
```
