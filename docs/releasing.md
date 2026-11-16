# Release-Prozess

Releases entstehen über den Workflow `.github/workflows/release.yml` (Windows, Rust 1.96.1). Er legt standardmäßig einen **Entwurf** (Draft) an; veröffentlicht wird manuell.

## Release erstellen

1. Version in `apps/desktop/Cargo.toml` anheben (und `Cargo.lock` aktualisieren). Der Tag muss exakt `v<Version>` lauten, sonst bricht der Workflow ab.
2. `scripts/package.ps1` liest die Version aus `apps/desktop/Cargo.toml` (`[package] version`, Helfer `scripts/common.ps1`) und erzeugt `dist/nexpanel-<Version>-windows-x64.zip` plus `.sha256`, passend zur Prüfung im Workflow. Das Paket enthält `VERSION.txt` und `THIRD-PARTY-NOTICES.md` (erzeugt von `scripts/gen-third-party-notices.ps1` aus `cargo metadata`, inkl. Lizenztexten aus der lokalen Cargo-Registry; die Registry-Quellen müssen vorhanden sein, ggf. vorher `cargo fetch --locked`).
3. Statuszeilen in `README.md`, `docs/testing.md` und `docs/package-readme.md` aktualisieren.
4. Änderungen per PR nach `main` bringen, CI muss grün sein.
5. Tag setzen und pushen: `git tag vX.Y.Z` und `git push origin vX.Y.Z`. Alternativ: Actions → Release → „Run workflow“ mit Tag (und `draft`).
6. Tag-Push erzeugt immer einen Entwurf. Nur der manuelle Lauf mit `draft = false` veröffentlicht direkt. Versionen unter 1.0 werden als Pre-release markiert.
7. Entwurf unter GitHub → Releases prüfen: Notizen (automatisch aus den Commits seit dem letzten Tag, plus fester Hinweisblock), Anhänge (`.zip` und `.sha256`).
8. Zip herunterladen und lokal prüfen:
   ```powershell
   (Get-FileHash .\nexpanel-X.Y.Z-windows-x64.zip -Algorithm SHA256).Hash.ToLower()
   Get-Content .\nexpanel-X.Y.Z-windows-x64.zip.sha256
   ```
   Beide Werte müssen übereinstimmen.
9. Entwurf veröffentlichen („Publish release“).

## Vor dem ersten öffentlichen Release

- [ ] Lizenz von NexPanel gewählt (`LICENSE`-Datei, siehe `docs/licensing.md`).
- [x] Drittlizenz-Datei im Paket (`THIRD-PARTY-NOTICES.md`, automatisch erzeugt).
- [ ] Lizenzen der Abhängigkeiten rechtlich geprüft (`docs/licensing.md`); Slint-Bedingungen und „Über NexPanel“ abgeglichen.
- [ ] Handtests aus `docs/testing.md` durchgeführt.
- [ ] Signierung entschieden: unsigniert (SmartScreen-Warnung) oder Zertifikat.
- [ ] Statusformulierung in `README.md` angepasst (kein „lokal, nicht gepusht“ mehr).
