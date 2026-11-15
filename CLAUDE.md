# MesDrivers : notes pour les agents

TUI Rust (ratatui) de gestion des pilotes Windows : inventaire WMI, mises à jour Windows Update + Catalogue Microsoft, DriverStore, sauvegarde/restauration, auto-update. Voir `docs/architecture.md` pour le détail.

## Commandes

```sh
cargo test                                   # logique pure, tourne sur macOS
cargo test -- --ignored                      # tests réseau réels (Catalogue, GitHub)
cargo clippy --all-targets -- -D warnings
cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings   # OBLIGATOIRE : seul moyen de compiler le code #[cfg(windows)] depuis le Mac
./target/debug/mesdrivers --catalog 'PCI\VEN_8086&DEV_51F1'                 # Catalogue en direct, marche hors Windows
```

Le code Windows (COM, WMI, SetupAPI, pnputil) ne peut **s'exécuter** que dans la CI. `.github/workflows/release.yml` lance `--scan`, `--catalog-get` et `--backup` sur `windows-latest` : après un push, lire leur sortie avec `gh run view <id> --log`.

## Règles du projet

- **Les deux clippy doivent passer avec `-D warnings`** (Mac et `x86_64-pc-windows-msvc`), comme en CI.
- **Code propre à Windows** derrière `#[cfg(windows)]`. Le reste (UI, `model.rs`, `catalog.rs`, `selfupdate.rs`) doit compiler et se tester partout. Toute logique non triviale va dans une fonction pure testée (voir `model.rs`, `catalog::parse`, `store::version_info`).
- **UI, messages et docs en français.** Pas d'emoji.
- **Threads :** pas de COM dans le thread UI. Les objets `IUpdate` ne sont pas `Send` et restent dans le worker (`worker.rs`). Nouvelle opération = nouvelle variante `Cmd`, le résultat revient en `Msg`, et chaque commande se termine par `Msg::Done`.
- **Touches :** ne traiter que `KeyEventKind::Press`, car Windows envoie aussi les relâchements.
- **Outils Windows** (`pnputil`, `expand`, `powershell`) : se fier au **code retour**, jamais au texte, qui est traduit selon la langue de Windows. Codes pnputil dans `docs/architecture.md`.
- **Opérations qui modifient le système** (installation, restauration, suppression) : point de restauration avant (`worker::restore_point`, repli via `Msg::NeedForce`), une ligne dans `journal::log`, confirmation `o/N` dans l'UI. Jamais de `pnputil /force`.
- **Catalogue Microsoft :** HTML non documenté (structure dans `docs/architecture.md`). Si le parsing change, ajouter un extrait réel en test dans `catalog.rs` (comme `ROW`). Le même paquet existe sous plusieurs ids : regrouper par (titre, version) et dédupliquer par empreinte SHA-1.
- **Dépendances :** n'en ajouter que si c'est vraiment nécessaire. TLS = `native-tls` (schannel) volontairement : pas de `ring`, qui casse la vérification croisée depuis le Mac.
- **Tester l'UI sans Windows :** placer un `cache.json` d'exemple dans `$TMPDIR/mesdrivers/` (structure : `cache::Cache`), puis piloter la TUI dans un pseudo-terminal avec `pyte` pour lire l'écran. Envoyer chaque séquence d'échappement d'un bloc : un `Échap` isolé quitte l'appli.
- **Dates Intel 1968** : convention normale des INF chipset. Les afficher via `model::display_date`, ne pas les « corriger ».

## Release

1. Passer `version` dans `Cargo.toml`, puis pousser sur `main` et attendre la CI verte.
2. `git tag vX.Y.Z && git push origin vX.Y.Z` : la CI publie `mesdrivers.exe` + `mesdrivers.exe.sha256`. L'auto-update s'appuie sur les deux, ne pas les renommer.
3. Dépôt : `PhilDaiguille/mesdrivers` (constante `REPO` dans `selfupdate.rs`).
