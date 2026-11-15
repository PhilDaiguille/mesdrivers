# MesDrivers

Détecte le matériel Windows, liste les pilotes installés et les périphériques en erreur, puis propose et installe les pilotes plus récents de **Windows Update** (sources officielles, signées Microsoft). Interface TUI dans le terminal.

Chaque pilote tiers est aussi vérifié dans le **Catalogue Microsoft Update**, qui a souvent des versions plus récentes que celles poussées par Windows Update (colonne « Catalogue MS », `o` pour ouvrir la page).

## Installation (PowerShell)

```powershell
irm https://github.com/PhilDaiguille/mesdrivers/releases/latest/download/mesdrivers.exe -OutFile $env:USERPROFILE\mesdrivers.exe
& $env:USERPROFILE\mesdrivers.exe
```

Windows demande les droits administrateur au lancement (nécessaires pour installer un pilote).

## Sécurité

- Un point de restauration Windows est créé avant chaque installation. S'il échoue (protection système désactivée), l'appli demande confirmation.
- Les pilotes sont installés un par un, avec la progression affichée.
- Chaque installation est notée dans `%LOCALAPPDATA%\mesdrivers\journal.txt`.

## Touches

| Touche | Action |
|---|---|
| `Tab` | changer d'onglet |
| `↑↓` `PgUp/PgDn` | naviguer |
| `/` | filtrer (Échap efface) |
| `m` | afficher/masquer les pilotes génériques Microsoft |
| `o` | ouvrir le Catalogue Microsoft pour la ligne sélectionnée |
| `Espace` / `a` | cocher / tout cocher (onglet Mises à jour) |
| `i` | installer la sélection |
| `r` | rescanner |
| `q` | quitter |

## Ligne de commande

```powershell
mesdrivers --scan                              # analyse complète sans interface, avec les durées
mesdrivers --catalog "PCI\VEN_8086&DEV_51F1"   # dernière version publiée au Catalogue
mesdrivers --version
```

## Publier une version

```sh
git tag v0.2.0 && git push --tags   # GitHub Actions teste, compile et attache mesdrivers.exe à la release
```
