# underground-2-android

Reimplementare nativă în Rust a motorului **Need for Speed: Underground 2** (PC, 2004), țintă principală Android (Samsung Galaxy S23 Ultra, Vulkan).

Nu emulăm nimic: nu rulăm `speed2.exe`, nu folosim Wine/Box64/x86. Motorul citește direct fișierele de date ale jocului (piste, mașini, texturi) și le randează cu propriul renderer (wgpu → Vulkan). Tot jocul e în `libug2.so`; singurul cod Java e o clasă mică (`MainActivity extends NativeActivity`) care ascunde barele de sistem.

> **Fișierele jocului NU sunt incluse și nu vor fi incluse în acest repo.** Ai nevoie de propria copie a jocului; fișierele se copiază pe telefon separat (vezi mai jos).

## Ce merge acum (v0.1)

- Orașul Bayview complet, încărcat din `TRACKS/STREAML4RA.BUN`: 583 de secțiuni, ~900k triunghiuri, ~1.900 de texturi, cer cu nori, neoane (blending aditiv), garduri și vegetație (alpha).
- Nissan 350Z stoc, cu roți poziționate din `CarTypeInfo` (GLOBALB), vopsea configurabilă.
- Condus: fizică arcade (accelerație, frână, mers înapoi, frână de mână cu drift, nitro, cutie automată cu 6 trepte), urmărirea terenului, coliziune cu clădiri, ziduri și parapeți.
- HUD tactil multi-touch: butoane « » pentru direcție (sau joystick virtual), GAS, BRAKE, NOS, E-BRAKE, LOOK (privire înapoi), CAM, RESET, pauză; vitezometru, treaptă, turație, nitro, FPS.
- Gamepad (Bluetooth/USB) și tastatură, cu mapare editabilă în `controls.cfg`.
- Testat pe Galaxy S23 Ultra (Adreno 740, Android 16): **~120 FPS** la ~1.800 de draw call-uri, orașul se încarcă în ~1 s.
- Testat și în emulatorul Android (x86_64, Vulkan): 40–50 FPS pe un GPU Intel integrat.

## Instalare pe telefon (S23 Ultra)

Pe PC: Android SDK + NDK (Android Studio), Rust (`rustup target add aarch64-linux-android`). Pe telefon: *Opțiuni dezvoltator → Depanare USB*.

```powershell
# 1. construiește și instalează APK-ul (build\ug2.apk)
powershell -ExecutionPolicy Bypass -File scripts\build-apk.ps1 -Install

# 2. copiază fișierele jocului pe telefon (~150 MB) din instalarea ta de PC
powershell -ExecutionPolicy Bypass -File scripts\push-game-data.ps1 -GameDir "D:\Jocuri\NFSU2"

# 3. pornește „NFS Underground 2” de pe telefon. Log-uri:
adb logcat -s ug2:*
```

Fișierele ajung în `/sdcard/Android/data/com.adrianstroescu.ug2/files/NFSU2/` (CARS, GLOBAL, TRACKS). Tot acolo se creează `ug2.cfg` (setări) și `controls.cfg` (mapare butoane).

`-Emulator` adaugă și biblioteca x86_64, ca același APK să meargă în emulatorul Android Studio.

## Controale

| Acțiune | Touch | Gamepad | Tastatură |
|---|---|---|---|
| Direcție | butoanele « » (sau joystick, din `ug2.cfg`) | stick stânga / D-pad | ← → / A D |
| Accelerație | GAS | R2 / R1 | ↑ / W |
| Frână / înapoi | BRAKE | L2 / L1 | ↓ / S |
| Frână de mână | E-BRAKE | X | Space |
| Nitro | NOS | A | Shift / N |
| Cameră | CAM | Y | C |
| Privire înapoi | LOOK | B | B |
| Reset mașină | RESET | Select | R |
| Pauză | II | Start | Esc / Back |

Degetul poate aluneca între GAS, BRAKE, NOS și E-BRAKE fără să fie ridicat de pe ecran.

## Setări (`ug2.cfg`)

`car` (ex. `350Z`, `SKYLINE`, `RX7`), `paint = r, g, b`, `steer_mode = buttons | joystick` (implicit butoane « », ca în jocurile de curse de mobil), `hud_scale`, `hud_opacity`, `msaa = 1 | 4`, `draw_distance`, `show_fps`.

## Structură

```
crates/
  ug2-formats   parsere pure: chunk-uri EA, JDLZ/HUFF, texturi TPK, geometrie, decor, CarTypeInfo
  ug2-engine    renderer wgpu, încărcare pe thread separat, fizică, coliziune, cameră, input, HUD
  ug2-android   NativeActivity: suprafață Vulkan, touch, gamepad, ciclul de viață al aplicației
  ug2-desktop   gazdă Windows/Linux pentru dezvoltare (fereastră, mouse ca touch, screenshot headless)
  ug2-tool      CLI de inspectare/export (OBJ + PNG) pentru validarea parserelor
android/        AndroidManifest.xml, MainActivity.java (ecran complet imersiv)
scripts/        build-apk.ps1, push-game-data.ps1
```

## Formate descifrate

| Format | Detalii |
|---|---|
| bChunk | `id u32, size u32`, container dacă bitul 31 e setat; padding `0x11` pentru aliniere |
| Compresie | JDLZ (LZ), HUFF (codec EA 0x30FB, Huffman canonic + RLE) |
| TPK | brut (0x33310004/5 + blob aliniat la 0x80) și comprimat (blocuri JDLZ/HUFF cu `TextureInfo` la final); DXT1/3/5, A8R8G8B8, P8; `AlphaBlendType` 1 = aditiv |
| Solid | header 0x134011 (hash, bbox, matrice, nume), vertecși 36 B (poz, normală, culoare, UV) sau 24 B (fără normală), triunghiuri u16, grupuri de material de 60 B |
| Decor | 0x34102 info (nume + 3 LOD-uri) 68 B, 0x34103 instanțe 64 B (bbox, info, flags, poziție, rotație i16/8192); flag 0x200 = bariere de cursă, 0x4/0x8 = grafica start/finish |
| Streaming | 0x34110 în `L4RA.BUN`: 393 de secțiuni de 80 B (nume, număr, offset, mărime) |
| CarTypeInfo | 0x34600 în GLOBALB, 0x890 B per mașină; roți la +0x120 (poziție, rază, lățime) |

## Urmează

Panoramele de fundal (`PAN_*`), streaming pe distanță (memorie), alte mașini și piese, faruri și reflexii pe mașină, trafic, sunet (motor, muzică), meniuri, curse și carieră. Build-ul de desktop crapă la pornire pe toolchain-ul `windows-gnu` (problemă de linkare a wgpu pe Windows, fără impact asupra Android).

## Licență

GPL-3.0-or-later. Decodorul HUFF urmează implementarea de referință EA publicată sub GPL-3.0 în codul sursă *Command & Conquer Generals*.

Need for Speed și Underground 2 sunt mărci Electronic Arts. Proiectul nu e afiliat EA și nu distribuie conținut EA.
