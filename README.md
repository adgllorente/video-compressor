# Compresor de vídeo

App de escritorio (macOS y Windows) para comprimir uno o varios vídeos a un tamaño final en MB.

- Selecciona vídeos con el botón o arrástralos a la ventana.
- Elige el tamaño final, el códec (H.264 o H.265), la velocidad y la resolución.
- Calcula el bitrate a partir de la duración y codifica en dos pasadas con ffmpeg. Si el resultado se pasa del objetivo, repite la segunda pasada con un bitrate menor.
- En modo «Automática», baja la resolución cuando el bitrate disponible es bajo (360p, 480p, 720p o 1080p).
- Guarda `nombre_25MB.mp4` junto al original o en la carpeta que elijas. Nunca sobrescribe.

## Descargar y usar

Descarga el zip de tu sistema desde [Releases](https://github.com/adgllorente/ra-video-compressor/releases/latest). Los paquetes ya llevan ffmpeg dentro; no hay que instalar nada.

- **macOS (Apple Silicon)**: `CompresorVideo-macos.zip` → descomprimir → arrastrar «Compresor de vídeo.app» a Aplicaciones. No está firmada con un certificado de Apple. La primera vez: clic derecho → Abrir → Abrir. Si dice que «está dañada», ejecuta `xattr -dr com.apple.quarantine "/Applications/Compresor de vídeo.app"`.
- **Windows 10/11 (x64)**: `CompresorVideo-windows.zip` → descomprimir la carpeta entera (el `.exe` necesita `ffmpeg.exe` al lado) → abrir «Compresor de video.exe». Si SmartScreen avisa: «Más información» → «Ejecutar de todas formas».

Si por lo que sea no encuentra ffmpeg (por ejemplo, si ejecutas el binario suelto), lo descarga solo la primera vez en `~/Library/Application Support/CompresorVideo` o en `%LOCALAPPDATA%\CompresorVideo`.

## Publicar una versión

Sube la versión en `Cargo.toml` y crea un tag:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

El workflow `Release` (`.github/workflows/release.yml`) compila macOS y Windows en máquinas nativas y publica los dos zips en la release. También se puede lanzar a mano desde la pestaña Actions; en ese caso deja los zips como artefactos, sin crear una release.

## Generar los paquetes en local

```sh
./scripts/package-macos.sh     # dist/CompresorVideo-macos.zip
./scripts/package-windows.sh   # dist/CompresorVideo-windows.zip
```

`package-windows.sh` compila desde el Mac con mingw (`brew install mingw-w64` y `rustup target add x86_64-pc-windows-gnu`). En Windows se puede lanzar desde Git Bash con `TARGET=x86_64-pc-windows-msvc`.

Si al compilar en macOS falla con `unknown architecture ... libSystem.tbd` (SDK más nuevo que el linker), apunta a un SDK anterior:

```sh
export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.sdk
```

## Icono

`assets/icon-source.png` es la imagen original. A partir de ella se generan `assets/icon-1024.png` (cuadrado, con margen), `assets/AppIcon.icns` (macOS), `assets/icon.ico` (Windows; `build.rs` lo incrusta en el `.exe`) y `assets/icon-256.png` (icono de la ventana).

## Desarrollo

```sh
cargo run
```
