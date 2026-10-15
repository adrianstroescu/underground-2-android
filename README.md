# underground-2-android

Reimplementare nativă în Rust a motorului **Need for Speed: Underground 2** (PC, 2004), țintă principală Android (Samsung Galaxy S23 Ultra, Vulkan), cu build de desktop pentru dezvoltare.

Nu emulăm nimic: nu rulăm `speed2.exe`, nu folosim Wine/Box64/x86. Motorul citește direct fișierele de date ale jocului (piste, mașini, texturi) și le randează cu propriul renderer (wgpu → Vulkan).

> **Fișierele jocului NU sunt incluse și nu vor fi incluse în acest repo.** Ai nevoie de propria copie a jocului; fișierele se copiază pe telefon separat (vezi mai jos).

## Stare

| Componentă | Stare |
|---|---|
| Chunk-uri EA (bChunk), hash `bStringHash` | ✅ |
| Decompresie JDLZ | ✅ |
| Decompresie HUFF (codec EA 0x30FB) | ✅ |
| Texture packs (TPK) brute + comprimate, DXT1/3/5, ARGB, P8 | ✅ |
| Geometrie (solid lists): vertecși, indecși, materiale | ✅ |
| Pistă: secțiuni de streaming, instanțe de decor (poziție + rotație) | ✅ |
| Mașini: piese stoc găsite prin hash | ✅ |
| Renderer wgpu (Vulkan pe Android) | 🚧 |
| HUD tactil: joystick virtual, pedale, nitro, frână de mână | 🚧 |
| Mapare controale (touch / gamepad / tastatură) | 🚧 |
| Fizică arcade + coliziune cu drumul | 🚧 |
| Audio, AI, curse, meniuri, carieră, tuning | ⏳ |

Validat pe datele originale: Bayview (`L4RA`): 393 secțiuni, 10.735 obiecte, ~900k triunghiuri, 1.836 texturi, 48.270 instanțe de decor, toate rezolvate.

## Structură

```
crates/
  ug2-formats   parsere pure (fără GPU): chunk-uri, JDLZ/HUFF, TPK, geometrie, pistă
  ug2-tool      CLI: inspectare și export (OBJ + PNG) pentru validarea parserelor
  ug2-engine    motorul: renderer, cameră, fizică, input, HUD
  ug2-desktop   aplicație Windows/Linux (winit) pentru dezvoltare rapidă
  ug2-android   aplicație Android (NativeActivity, fără Java)
```

## Build

Cerințe: Rust stable, Android SDK + NDK (r27+), `rustup target add aarch64-linux-android`.

```sh
# unealta de inspectare
cargo run -p ug2-tool --release -- track "D:/Jocuri/NFSU2"
cargo run -p ug2-tool --release -- car "D:/Jocuri/NFSU2" 350Z out/
```

> Pe Windows cu toolchain-ul `x86_64-pc-windows-gnu` (fără Visual Studio), setează
> `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C dlltool=<NDK>/toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-dlltool.exe"`.

## Licență

GPL-3.0-or-later. Decodorul HUFF urmează implementarea de referință EA publicată sub GPL-3.0 în codul sursă *Command & Conquer Generals*.

Need for Speed și Underground 2 sunt mărci Electronic Arts. Proiectul nu e afiliat EA și nu distribuie conținut EA.
