# Release-Prozess

Releases entstehen über den Workflow `.github/workflows/release.yml` (Windows, Rust 1.96.1). Er legt standardmäßig einen **Entwurf** (Draft) an; veröffentlicht wird manuell.

## Release erstellen

1. Version in `apps/desktop/Cargo.toml` anheben (und `Cargo.lock` aktualisieren). Der Tag muss exakt `v<Version>` lauten, sonst bricht der Workflow ab.
2. `scripts/package.ps1` benennt das Paket derzeit fest `nexpanel-0.1.0-windows-x64.zip`. Bei einer neuen Version den Dateinamen im Skript anpassen (der Workflow prüft `dist/nexpanel-<Version>-windows-x64.zip` und schlägt sonst fehl).
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
- [ ] Lizenzen der Abhängigkeiten geprüft, auch transitive (`docs/licensing.md`, z. B. `cargo-deny`/`cargo-about`); Slint-Bedingungen und „Über NexPanel“ abgeglichen.
- [ ] Handtests aus `docs/testing.md` durchgeführt.
- [ ] Signierung entschieden: unsigniert (SmartScreen-Warnung) oder Zertifikat.
- [ ] Statusformulierung in `README.md` angepasst (kein „lokal, nicht gepusht“ mehr).
