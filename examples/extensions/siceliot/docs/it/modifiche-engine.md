# Modifiche all'engine

Cosa cambia SICELIOT in Lumit (`D:\Siceliot\Lumit`): quattordici commit sul ramo
`siceliot/lumit-0.10`, sopra `v0.10.0` originale (`eb27a6eb`, Lumit 0.10.0), tenuti anche come
`patches/0001…0014` ([README](../../patches/README.md)). Per ogni voce: perché c'è, cosa fa oggi
e quali file tocca.

*English: [../en/engine-changes.md](../en/engine-changes.md)*

## Come si verifica

- `cargo test -p lumit-core --lib`: 579 superati. `cargo test -p lumit_bridge --lib`: 246 superati.
- `cargo clippy -p lumit-core -p lumit-render -p lumit_bridge --all-targets -- -D warnings`,
  `cargo fmt --check`: puliti.
- `flutter test`: tutto passa tranne `ae_import_frb_test` ("a real .aep imports through the front
  door"), che fallisce allo stesso modo su `v0.10.0` originale senza modifiche.
- `sh extensions/src/build.sh`: i 19 effetti LFX superano `tools/lfx-check` sotto Siceliot FX.
- Render con `lumit-cli`: `tests/render` (ogni effetto LFX), `tests/copies` (Clone to points per
  copia, etichette decode, lo sketch ad anelli), `tests/drivers` e i due progetti di esempio.
- Tutte e quattordici le patch si applicano su `v0.10.0` con `git am` e danno lo stesso albero del ramo.

| # | Commit | Modifica |
|---|---|---|
| 1 | `313fbc5a` | [Una categoria tutta di Siceliot](#1-una-categoria-tutta-di-siceliot) |
| 2 | `9863e783` | [Driver: Stagger, Random, Oscillator, Value noise](#2-driver-stagger-random-oscillator-value-noise) |
| 3 | `4b5637f0` | [Scale, Rotation e Opacity per copia in Clone to points](#3-scale-rotation-e-opacity-per-copia-in-clone-to-points) |
| 4 | `c3133fd9` | [Oscillator e Value noise variano con la posizione](#4-oscillator-e-value-noise-variano-con-la-posizione) |
| 5 | `d547d237` | [Righe grigie nei driver e testo casuale in Label points](#5-righe-grigie-nei-driver-e-testo-casuale-in-label-points) |
| 6 | `c7405e86` | [Decode: Label points reagisce a un'onda](#6-decode-label-points-reagisce-a-unonda) |
| 7 | `495e1cf5` | [Anelli: Ring wave ed etichette fluide che ruotano](#7-anelli-ring-wave-ed-etichette-fluide-che-ruotano) |
| 8 | `183f23e5` | [Colori Gradient, Away from points, Grow, Vertical anchor](#8-colori-gradient-away-from-points-grow-vertical-anchor) |
| 9 | `4c87457e` | [Gli effetti copiati portano con sé driver e collegamenti](#9-gli-effetti-copiati-portano-con-sé-driver-e-collegamenti) |
| 10 | `4c8c67b0` | [Type fill](#10-type-fill) |
| 11 | `425fa936` | [Due intestazioni: Siceliot FX e Siceliot Nodes](#11-due-intestazioni-siceliot-fx-e-siceliot-nodes) |
| 12 | `48ed40b7` | [Type fill: ogni parola una volta per colonna, e Cut words](#12-type-fill-ogni-parola-una-volta-per-colonna-e-cut-words) |
| 13 | `95cfc997` | [Backdrop: un effetto LFX che piega ciò che sta dietro al suo livello](#13-backdrop-un-effetto-lfx-che-piega-ciò-che-sta-dietro-al-suo-livello) |
| 14 | `f3e3d364` | [Righe LFX in grigio: @enabled_if](#14-righe-lfx-in-grigio-enabled_if) |

Ogni modifica che aggiunge una riga o un nome aggiorna anche `flutter_ui/lib/l10n/app_en.arb`,
`engine_labels.dart` e i file generati `crates/lumit-core/fx-labels.txt` e `fx-reference.json`
(`cargo test -p lumit-core --lib -- regenerate_fx_label_fixture regenerate_fx_reference
--ignored`). Gli altri file di lingua non si modificano a mano, come chiedono le regole di Lumit.

---

## 1. Una categoria tutta di Siceliot

Lumit mette ogni effetto in una categoria predefinita, e un manifest LFX può indicare solo una
delle otto categorie di immagine; qualsiasi altro nome viene rifiutato con *"`…` is not one of
Lumit's effect categories"*. Quindi l'engine deve conoscere la categoria di Siceliot.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/schema.rs` | `FxCategory::Siceliot`, in `FxCategory::ALL` subito dopo `Utility`, così è l'ultima famiglia di immagine in ogni menu. Etichetta **Siceliot FX** (modifica 11) |
| `crates/lumit-core/src/fx/lfx.rs` | `Siceliot` in `PICTURE_CATEGORIES`, così un manifest può indicarla (maiuscole ignorate). Il test del manifest la controlla |
| `crates/lumit-core/src/edits.rs` | Chiave `siceliot`, con cui il browser degli effetti raggruppa e ordina |
| `flutter_ui/lib/l10n/app_en.arb`, `engine_labels.dart` | `fxSiceliot`, un nome proprio che resta uguale in ogni lingua |

Un'estensione la indica così; si installa solo in un Lumit compilato con queste modifiche:

```json
"effects": [
  { "id": "falloff", "name": "Falloff", "category": "Siceliot FX",
    "version": 1, "api": 1, "shader": "falloff.wgsl" }
]
```

---

## 2. Driver: Stagger, Random, Oscillator, Value noise

La modularità di Cavalry si basa su behaviour collegati a qualsiasi attributo e calcolati una
volta per copia. Un effetto LFX può solo produrre immagini, quindi i driver, che producono numeri,
sono costruiti nell'engine. Lumit dà già a ogni copia di Clone to points il suo numero (Clone
index); questi driver lo trasformano in Stagger, Random, Oscillator e Noise di Cavalry. Vedi
[effetti.md](effetti.md#siceliot-nodes).

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/drivers/siceliot.rs` | I driver (`siceliot_stagger`, `siceliot_random`, `siceliot_oscillator`, `siceliot_noise`), la loro matematica come funzioni pubbliche e un test dei valori promessi dalla documentazione |
| `crates/lumit-core/src/fx/schema.rs` | `FxCategory::SiceliotDrivers` (mostrata come **Siceliot Nodes**, modifica 11) e `FxCategory::is_driver()`, vero per `Drivers` e `SiceliotDrivers` |
| `crates/lumit-core/src/comp_graph.rs`, `crates/lumit-eval/src/lib.rs`, `crates/lumit-script/src/commands/effects.rs`, `crates/lumit-bridge/src/api/{composition,effect,layer}.rs` | Ogni controllo "è un driver?" usa `is_driver()`, così i driver di Siceliot vanno nel node graph, si collegano ai parametri e restano fuori dalla pila esattamente come quelli di Lumit |
| `crates/lumit-core/src/fx/drivers/clone_index.rs` | Clone to points renderizza un layer una volta per copia quando vi trova Clone index **o** un driver di Siceliot |
| `crates/lumit-core/src/fx/catalogue.rs`, `drivers/mod.rs`, `registry.rs`, `edits.rs`, `tests.rs` | Registrazione, chiave di categoria `siceliot_drivers`, il test del matte che esclude ogni categoria di driver |

Il Noise di Cavalry si chiama **Value noise**, perché Lumit ha già un effetto chiamato Noise.

---

## 3. Scale, Rotation e Opacity per copia in Clone to points

Un effetto Transform sul layer clonato ritaglia le copie: l'immagine di un layer di forme è grande
solo quanto il disegno, e un effetto non può disegnare oltre. In Cavalry un behaviour collegato a
Shape Rotation del Duplicator ruota ogni copia per intero. Clone to points ruota e scala già ogni
timbro secondo il suo punto; prende anche il valore per timbro dai driver.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/effects/clone_to_points.rs` | `drive_rows`: quando **Scale**, **Rotation** o **Opacity** sono collegati e il graph del layer contiene un driver che legge la copia, i driver si risolvono una volta per punto come copia *i* di *n*, nelle colonne `siceliot:scale`, `siceliot:rotation`, `siceliot:opacity`. `stamps()` le legge timbro per timbro. Al massimo 4096 punti sono calcolati uno per uno; oltre si ripete l'ultimo valore. Distribuito sui core |
| `crates/lumit-render/src/build.rs` | `points_input_for` lo esegue sul flusso passato a Clone to points (con il graph delle comp a nodi) |
| `crates/lumit-core/src/fx/drivers/clone_index.rs` | `reads_copy` pubblico |
| `crates/lumit-core/src/fx/tests.rs`, `drivers/mod.rs` | Uno Stagger (da 0 a 90) collegato a Rotation ruota tre punti di 0°, 45°, 90° |

---

## 4. Oscillator e Value noise variano con la posizione

Con Phase per copy un'onda segue il numero delle copie, quindi su una Grid scorre riga per riga.
In Cavalry un'onda può attraversare le copie nello spazio. I driver sanno dove si trova ogni copia.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/registry.rs` | `DriverCx::point`: dove si trova la copia, in px@comp |
| `crates/lumit-core/src/fx/drivers/mod.rs` | Il percorso dei driver porta il punto; `resolve_drivers_at_point` |
| `crates/lumit-core/src/fx/drivers/siceliot.rs` | **Oscillator**: **Phase from** (Copy · Position), **Wave angle**, **Wavelength**. **Value noise**: **Vary by** (Nothing · Copy · Position), **Noise scale**. Test |
| `crates/lumit-core/src/fx/tests.rs` | Le nuove righe in px |

La posizione è nota quando il driver è collegato a Scale, Rotation o Opacity di Clone to points
(modifica 3) o alle righe pilotate di Label points (modifiche 6, 7); altrove vale l'angolo in alto
a sinistra della comp.

---

## 5. Righe grigie nei driver e testo casuale in Label points

Lumit rende grigie le righe che una modalità non usa; i driver lo fanno anche loro. E Label points,
che sapeva scrivere solo numeri, può mettere testo casuale su ogni punto, come lo String Generator
di Cavalry su un Duplicator.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/drivers/siceliot.rs` | `OSCILLATOR_ENABLED_WHEN`, `VALUE_NOISE_ENABLED_WHEN`: Phase per copy su Copy, Wave angle e Wavelength su Position, Noise scale su Position |
| `crates/lumit-core/src/fx/effects/label_points.rs` | `random_text`: `{pick:a\|b\|c}`, `{chars:N}` e `{hex:N}` sostituiti per punto, secondo l'id del punto e la riga **Seed**, così ogni punto tiene il suo testo in qualsiasi ordine. Un test |
| `web-docs/…/effects/generate/label-points.mdx` | Il manuale di Lumit: i segnaposto e Seed |

---

## 6. Decode: Label points reagisce a un'onda

L'effetto "testo che si decodifica": una griglia di caratteri il cui testo cambia e il cui colore
lampeggia mentre un'onda la attraversa, per tornare a riposo quando è passata.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/effects/label_points.rs` | Righe **Change**, **Change above**, **Scramble rate**, **Flash colour** (Off · Random hue · Two colours, più le opzioni delle modifiche 7 e 8), **Colour A**, **Colour B**, **Label scale**. Sopra Change above il seme dei segnaposto avanza Scramble rate volte al secondo e si estrae un colore per punto e per passo. L'effetto è `seeded`, quindi la cache dei fotogrammi lo indicizza per tempo. Un test |
| `crates/lumit-render/src/build.rs` | I driver collegati alle righe di `LABEL_DRIVEN_ROWS` (Change, Label scale, e Text step e Label rotation dalla modifica 7) si risolvono per punto con `drive_rows` |
| `web-docs/…/effects/generate/label-points.mdx` | Il manuale di Lumit: le nuove righe |

---

## 7. Anelli: Ring wave ed etichette fluide che ruotano

Per raggiungere uno sketch Processing ("lettere su lettere": una lettera gigante piena di
caratteri da 12 px, attraversata da anelli che li colorano, ingrandiscono, ruotano e cambiano) con
la stessa fluidità.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/drivers/siceliot.rs` | Driver **Ring wave** (`siceliot_ring_wave`): Minimum, Maximum, Interval, Speed, Speed variance, Thickness min, Thickness max, Origin, Spin, Seed; uscite **Value**, **Rotation**, **Passes** (e **Grow**, modifica 8). L'anello `k` parte a `k` × Interval e vive finché non ha attraversato il quadro, così Passes si conta dal solo tempo. In `MATCH_NAMES`, quindi si risolve per punto. Un test |
| `crates/lumit-core/src/fx/effects/label_points.rs` | Righe **Text step** (ogni numero intero fa avanzare il seme a riposo), **Hue start**, **Hue range**, **Saturation**, **Darken** (per Flash colour **By strength**), **Grow**, **Label rotation**. Un'etichetta ha una rotazione e una tinta (Replace per i lampi, Multiply per By strength). Test |
| `crates/lumit-core/src/fx/points.rs`, `crates/lumit-render/src/build.rs`, `fxops.rs` | `PointsSchedule::late_drive`: un flusso creato sulla scheda (Pick points per Image) riceve i suoi driver per punto quando arriva; anche per il Particulate con Trails di 0.10 |
| `crates/lumit-render/src/gpufx.rs` | Ogni testo si compone una volta per mezza ottava di dimensione e ogni etichetta si ricampiona da lì, bilineare, nella sua posizione frazionaria, dimensione e rotazione, poi si tinge. La composizione è distribuita sui core |
| `web-docs/…/effects/generate/label-points.mdx` | Il manuale di Lumit: le nuove righe |

Il driver si chiama Ring wave perché Lumit ha già un **Ripple** (una distorsione). Su una RTX 3060,
845 lettere a 1920 × 1080 costano a Label points circa 14 ms a fotogramma.

---

## 8. Colori Gradient, Away from points, Grow, Vertical anchor

Più vicino allo sketch: un gradiente di colori a scelta, anelli che non partono mai sulla lettera,
lettere che crescono della quota di ogni anello, anelli sovrapposti che si sommano, lettere appese
per la parte alta.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/effects/label_points.rs` | Flash colour **Gradient**: **Gradient stops** (da 2 a 4), da **Gradient 1** a **Gradient 4**, mescolati in luce lineare secondo Change, moltiplicando il colore del Text layer. **Vertical anchor** (Middle · Top · Bottom). By strength fa girare la tinta oltre Change 100. Test |
| `crates/lumit-core/src/fx/drivers/siceliot.rs` | Ring wave: Origin **Away from points** con **Clearance** (fino a 64 tentativi, secondo il seme, per un punto così lontano da ogni punto); **Add up**; **Grow min**, **Grow max** e l'uscita **Grow**. Test |
| `crates/lumit-core/src/fx/points.rs`, `registry.rs`, `drivers/mod.rs`, `effects/clone_to_points.rs` | `Cloud`: i punti del flusso divisi in celle; creato una volta da `drive_rows` e portato dal percorso dei driver accanto al punto |
| `crates/lumit-render/src/gpufx.rs` | Label points passa entrambi gli ancoraggi |
| `web-docs/…/effects/generate/label-points.mdx` | Il manuale di Lumit: le nuove righe |

---

## 9. Gli effetti copiati portano con sé driver e collegamenti

Copiare effetti collegati (Grid, Pick points, Label points) e incollarli su un altro layer dava un
layer che non disegnava nulla: i collegamenti e i driver che li pilotano stanno nel node graph del
layer e restavano indietro.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/preset.rs` | `EffectPreset::wiring`: `wiring_of` raccoglie i collegamenti tra gli effetti copiati e ogni driver che li pilota, direttamente o attraverso altri driver; `None` se non c'è nulla di collegato. `to_json_wired`. Un test |
| `crates/lumit-bridge/src/api/layer.rs` | `copy_effects` aggiunge i collegamenti; `paste_effects` ricrea effetti, driver e collegamenti con id nuovi, a destra di ciò che il graph di destinazione contiene, in un solo commit, con i keyframe sotto la testina |
| `crates/lumit-bridge/src/api/tests.rs` | Un Blur pilotato da un Wiggle arriva con il suo Wiggle collegato a Radius |

I preset `.lumfx` salvati contengono solo la pila, e un Lumit senza questa modifica legge gli
effetti di un testo copiato e ignora i collegamenti.

---

## 10. Type fill

Scene di Cavalry come *Submesh Boolean* di Tim Siegert riempiono il quadro di righe di parole che
si allungano nello spazio lasciato da una forma in movimento. Lumit non sapeva disporre testo
secondo una regione, e uno shader LFX non può disegnare testo, quindi **Type fill** è nell'engine.
Vedi [effetti.md](effetti.md#type-fill).

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/effects/type_fill.rs` | Effetto `siceliot_type_fill`: Text layer, Words, Region layer, Region placement, Region from, Invert region, Threshold, Rows, Row gap, Word gap, Min width, Order, Seed, Mix (Cut words: modifica 12). La disposizione (fasce di righe, colonne larghe quanto le parole, tratti liberi lungo ogni riga) è Rust puro con un test |
| `crates/lumit-core/src/fx/registry.rs`, `points.rs` | `EffectDef::reads_text_layer()`, `points::wants_carriage_of(def)`: un effetto può chiedere il documento del suo Text layer senza prendere punti |
| `crates/lumit-render/src/build.rs`, `fxops.rs` | Il documento di testo passa attraverso `wants_carriage_of` |
| `crates/lumit-render/src/gpufx.rs` | Rilegge il Region layer, lo campiona lungo il centro di ogni riga, disegna ogni parola rifilata e scalata sul suo pezzo (`stamp_box`, in comune con Label points) |
| `crates/lumit-core/src/fx/effects/label_points.rs` | `Placed::boxed`, `LabelDrawing::trimmed`, scala per asse |
| `catalogue.rs`, `effects/mod.rs`, `tests.rs`, `web-docs/…/generate/type-fill.mdx` | Registrazione, righe in px, pagina del manuale |

---

## 11. Due intestazioni: Siceliot FX e Siceliot Nodes

Effetti di immagine e driver stavano sotto un'unica intestazione, e niente diceva che i driver
funzionano solo nel Node graph. Le intestazioni sono due:

- **Siceliot FX**: effetti che si aggiungono a un layer come ogni altro, in Effects & presets e nel
  menu Add effect.
- **Siceliot Nodes**: i driver, elencati solo nel Node graph.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/schema.rs` | `FxCategory::Siceliot` con etichetta `Siceliot FX`; `FxCategory::SiceliotDrivers` con etichetta `Siceliot Nodes`, un'intestazione a sé |
| `crates/lumit-core/src/fx/lfx.rs` | Un manifest indica `"Siceliot FX"`; `"SICELIOT"` vale come la stessa categoria, così le estensioni 1.2.0 continuano a funzionare. Il test controlla entrambi |
| `flutter_ui/lib/l10n/app_en.arb`, `engine_labels.dart` | `fxSiceliot`, `fxSiceliotNodes` |
| `web-docs/…/type-fill.mdx`, `label-points.mdx` | Il manuale dice Siceliot FX |

---

## 12. Type fill: ogni parola una volta per colonna, e Cut words

Una forma che tagliava una colonna in due dava la sua parola a entrambe le parti, così compariva
un secondo "the" che saltava qua e là. Dove il bordo di una forma attraversava una colonna, i Type
fill dentro e fuori allungavano ognuno la parola nella propria parte. E poiché la regione si legge
solo lungo il centro di ogni riga, una riga alta faceva uscire le parole da un bordo curvo: bianco
sul bianco, nero sul nero.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/effects/type_fill.rs` | `TypeFill::pieces`: la parola di una colonna si allunga dalla prima parte libera all'ultima, se coprono almeno **Min width**, e ogni parte riceve un `Piece` con la sua fetta (`clip`). Riga **Cut words** (`cut_words`, spenta): ogni parola intera, larga quanto la sua colonna. `TypeFill::coverage`: un gradino morbido a Threshold, così un Type fill e il suo gemello con Invert region si dividono esattamente ogni pixel del bordo. Test |
| `crates/lumit-core/src/fx/effects/label_points.rs` | `Placed::clipped_x`: si disegna solo una fetta di una parola |
| `crates/lumit-render/src/gpufx.rs` | Ogni pezzo si disegna ritagliato sulla sua fetta, poi le parole sono mascherate dalla regione pixel per pixel (`TypeFill::coverage`), con Cut words acceso o spento |
| `web-docs/…/generate/type-fill.mdx` | Il manuale: Cut words, Min width, il taglio per pixel |

---

## 13. Backdrop: un effetto LFX che piega ciò che sta dietro al suo livello

Un effetto LFX su un livello con un'immagine vedeva solo quell'immagine. Un vetro, una lente o
qualunque cosa pieghi ciò che sta dietro una forma andava messa su un livello di regolazione,
scegliendo la forma in una riga. Un effetto il cui manifest dice `"backdrop": true` ora funziona
direttamente sul livello: per quel fotogramma il livello è disegnato come una regolazione, sopra il
composito di quello che c'è sotto, e la sua immagine è il secondo input dell'effetto.

- Il livello copre tutto il fotogramma, senza spostamenti, con la sua opacità. Le sue maschere e la
  sua trasformazione danno forma all'immagine passata all'effetto, messa dove il livello sta nella
  comp (come gli input `depth`).
- Una riga che sceglie un altro livello lo tiene. I pixel del livello non vengono disegnati.
- Anche ogni altro effetto sul livello lavora sul composito, come su un livello di regolazione.
- Nel progetto il livello resta quello che era: la sua immagine viene ancora decodificata, e su un
  livello di regolazione non cambia nulla. Il Lumit ufficiale ignora la riga, quindi la stessa
  estensione lì funziona su un livello di regolazione.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/registry.rs` | `EffectDef::backdrop`, falso di base |
| `crates/lumit-core/src/fx/lfx.rs` | `LfxEffect::backdrop`, tenuto nella chiave di forma della def, così una nuova scansione lo cambia. Test |
| `crates/lumit-extensions/src/lib.rs` | `ManifestEffect::backdrop` (`#[serde(default)]`) |
| `crates/lumit-render/src/build.rs` | `backdrop_fx`: un livello con un effetto backdrop attivo prende la strada delle regolazioni e copre la comp senza trasformazione, maschere, genitore né 3D. `own_shape`: l'immagine del livello prima dei suoi effetti, posizionata con `world_placement`, come input di quell'effetto quando la sua riga è vuota o nomina il livello |
| `web-docs/…/use/lfx-effects.mdx` | Il manuale: il campo `backdrop` |

---

## 14. Righe LFX in grigio: @enabled_if

Le righe di uno shader erano tutte attive, anche quelle che nella situazione attuale non facevano
nulla: Reaizzi Glass su un PNG lasciava modificabili Centre, Width, Height, Corner radius e Shape
rotation della forma integrata, anche se la forma la dà il PNG. Ora un'annotazione di riga dice
quando una riga ha effetto, e Effect controls altrimenti la mette in grigio.

- `@enabled_if(termine, …)`: attiva finché vale almeno un termine. Più annotazioni sulla stessa
  riga devono valere tutte.
- Termini: `input` / `!input`, se il secondo input dà un'immagine (la sua riga nomina un livello,
  oppure un effetto backdrop sta su un livello con un'immagine); `campo=N` / `campo!=N`, una
  scelta sull'opzione N o un toggle 0 o 1.
- Un termine che non si legge fa cadere la sua regola (la riga resta attiva) con una nota. Lo
  shader non ne sa nulla, e il Lumit ufficiale ignora l'annotazione.

| File | Modifica |
|---|---|
| `crates/lumit-core/src/fx/shader/mod.rs` | `RowTerm`; `ShaderField::enabled_if`, letto dalle annotazioni del campo |
| `crates/lumit-core/src/fx/registry.rs` | `EffectDef::row_state_on(inst, picture)`, di base `row_state` |
| `crates/lumit-core/src/fx/lfx.rs` | `LfxDef::row_state_on`: le righe le cui regole non valgono, in grigio. Test |
| `crates/lumit-bridge/src/api/effect.rs`, `layer.rs`, `composition.rs` | `read_instance_info` sa se il livello ha un'immagine (`!is_adjustment()`) |
| `web-docs/…/use/lfx-effects.mdx` | Il manuale: `@enabled_if` |
