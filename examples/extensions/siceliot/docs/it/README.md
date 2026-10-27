# SICELIOT — documentazione (italiano)

SICELIOT aggiunge a [Lumit](https://github.com/luminalmvm/Lumit) effetti non distruttivi in stile
Cavalry. Gli effetti di immagine compaiono sotto **Siceliot FX** nel menu Add effect, nel
pannello Effects & presets e nella ricerca dei nodi; i driver sotto **Siceliot Nodes**, solo nel
Node graph.

*English: [../en/README.md](../en/README.md)*

| Pagina | Contenuto |
|---|---|
| [modifiche-engine.md](modifiche-engine.md) | Ogni modifica fatta al motore e all'interfaccia di Lumit, file per file |
| [compilazione.md](compilazione.md) | Preparare un PC Windows e compilare il Lumit modificato |
| [effetti.md](effetti.md) | Siceliot FX e Siceliot Nodes a confronto; ogni effetto e nodo: parametri, un esempio d'uso, l'equivalente in Cavalry |
| [esempi.md](esempi.md) | Due progetti di esempio, Flood e Sketch |
| [../SICELIOT.md](../SICELIOT.md) | Tutto in una pagina: ogni effetto, ogni modifica all'engine, come aggiungere SICELIOT a Lumit |

## Cartelle

| Cartella | Contenuto |
|---|---|
| `extensions\siceliot` | L'estensione LFX SICELIOT come la installa Lumit (shader generati) |
| `extensions\src` | I sorgenti degli shader e `build.sh`, che scrive e verifica l'estensione |
| `dist\siceliot.lfx` | L'estensione in un unico pacchetto, per *Settings ▸ Addons ▸ Extensions* |
| `tools\lfx-check` | Verifica un'estensione come la legge Lumit e valida il WGSL con naga |
| `examples` | I progetti di esempio (Flood, Sketch), lo script che crea Flood e quello che li renderizza |
| `tests\render` | Script che renderizzano ogni effetto con `lumit-cli` |
| `tests\copies`, `tests\drivers` | Script che verificano i driver e Clone to points per copia con `lumit-cli` |
| `patches` | Le modifiche all'engine come file `git format-patch`, da applicare a Lumit originale |
| `docs/research` | Note di ricerca su Cavalry e Lumit, in inglese e in italiano |
| `Lumit` | Il sorgente di Lumit con le modifiche all'engine (non in questo repository) |

## Convenzioni

- Tutto ciò che compare dentro Lumit è in **inglese**: nomi degli effetti, etichette dei parametri,
  la categoria, i manifest e i commenti degli shader. In questa documentazione i nomi restano
  quindi in inglese, scritti esattamente come nel programma.
- Lumit scrive le etichette con la sola iniziale maiuscola. Le intestazioni **Siceliot FX** (effetti di
  immagine) e **Siceliot Nodes** (driver, solo nel Node graph) mantengono il nome proprio Siceliot
  in tutte le lingue.
- Il sorgente di Lumit è in `D:\Siceliot\Lumit`. Il lavoro è sul ramo Git `siceliot/lumit-0.10`,
  le quattordici modifiche all'engine sopra Lumit 0.10.0 originale; `main` resta identico al Lumit
  originale.
