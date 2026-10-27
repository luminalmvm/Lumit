# Cavalry vs Lumit — il modello non distruttivo

*English: [../en/cavalry-vs-lumit.md](../en/cavalry-vs-lumit.md)*

> **Note di ricerca del 26 ottobre 2026**, scritte prima di qualsiasi codice SICELIOT. Spiegano il
> ragionamento alla base del progetto; alcune parti oggi sono superate (per esempio Lumit 0.9.1
> supporta LFX, e molti degli effetti "mancanti" qui sotto ora esistono in SICELIOT). Per ciò che
> è stato davvero costruito vedi [../../it/effetti.md](../../it/effetti.md) e
> [../../it/modifiche-engine.md](../../it/modifiche-engine.md).

Fonti: documentazione Cavalry ([cavalry.studio/docs](https://cavalry.studio/docs/)) e documentazione + codice sorgente di Lumit ([github.com/luminalmvm/Lumit](https://github.com/luminalmvm/Lumit), ramo `main` al 26/10/2026). L'elenco completo dei nodi Cavalry è in [cavalry-effetti-e-modificatori.md](cavalry-effetti-e-modificatori.md); l'elenco grezzo in [../cavalry-nodes.tsv](../cavalry-nodes.tsv).

---

## 1. Come funziona Cavalry

### 1.1 L'idea di fondo: tutto resta un parametro vivo
In Cavalry niente viene "applicato" e congelato. Ogni forma, deformer, shader e filtro è un **layer con attributi**, e il risultato si ricalcola a ogni frame partendo dalla sorgente. Per modificare qualcosa basta cambiare un attributo, staccare un collegamento o riordinare una lista. Non c'è nessun "bake", a meno di non chiederlo esplicitamente.

Sotto l'interfaccia a layer c'è un **grafo di nodi**: i collegamenti passano dati da un attributo di un layer a quello di un altro, senza espressioni, quindi in modo veloce.

### 1.2 I quattro meccanismi che lo rendono modulare

**A. Collegamenti tra attributi** ([Connections](https://cavalry.studio/docs/getting-started/key-concepts/connections/))
- Qualsiasi attributo può pilotare qualsiasi altro attributo compatibile (stesso tipo di dato), **anche tra layer diversi e anche dentro altri behaviour**. Per esempio, un Random può pilotare il `maximum` di uno Stagger, che a sua volta pilota lo `shapeScale` di un Duplicator.
- Ogni attributo ha **un solo input e infiniti output**. I keyframe contano come input, quindi un collegamento sostituisce l'animazione.
- Ogni layer ha un'uscita `id`. Collegare l'`id` passa *l'intero nodo*, non un numero: è così che un deformer entra nella lista Deformers o uno shader in un materiale.

**B. Il contesto per indice** ([Context](https://cavalry.studio/docs/getting-started/key-concepts/context/), [Index Context](https://cavalry.studio/docs/nodes/utilities/index-context/))
- Il Duplicator chiede ogni valore collegato **una volta per copia**, passando l'indice della copia. Tutto ciò che sta a monte (Random, Stagger, Noise, Color Array, String Generator, perfino il numero di lati di un poligono o il raggio di un blur) viene valutato di nuovo per ogni copia.
- L'indice attraversa i **Duplicator annidati**, e con **Index Context → Depth** si sceglie di quale livello leggerlo. `Use Index Context` lo spegne.
- Altri "contesti" funzionano allo stesso modo: lunghezza del path (Length Context), velocità della copia (Velocity Context), seed per collegamento (Layer Seed).
- Gli **Array** (Value, Color, Shape, Shader, Asset, String Array) assegnano un elemento per indice. Così una sola rete di nodi produce copie tutte diverse.

**C. Pile ordinate con blend mode**: ogni stadio è una lista riordinabile, e ogni elemento sceglie come fondersi con quelli sotto.

| Pila | Dove | Come si combinano |
|---|---|---|
| **Falloff** | su ogni Behaviour / Field | Normal (Add), Min, Max, Minus, Multiply, Screen, Overlay, dal basso verso l'alto |
| **Behaviour Mixer** | combina behaviour di valore | stessi modi, più Strength ed Enabled per indice (non accetta deformer) |
| **Deformers** | lista sulla forma | in ordine, ciascuno con la sua Strength e i suoi Falloff |
| **Shaders** | nei materiali Fill / Stroke | blend mode + opacità; Alpha del materiale come master |
| **Filters** | lista sulla forma | blend mode + opacità + **Mattes** (luma/alpha, feather, *Filter Input* o *Clip Output*) |

Il **Falloff** ([doc](https://cavalry.studio/docs/nodes/utilities/falloff/)) è il "dove" di ogni effetto: forma Circle / Rectangle / Linear / Sweep / Shape (riempita o bordi), un grafo che decide la curva di decadimento, una Probability per renderlo binario, e i layer mode per comporne più d'uno.

**D. Incapsulamento e riuso**
- **Pre-comp** e **Pre-Comp Overrides** ([doc](https://cavalry.studio/docs/nodes/shapes/composition/pre-comp-overrides/)): una composizione sorgente, tante istanze, e per ogni istanza si cambiano solo gli attributi marcati come override. È il sistema dei template.
- **Component**: raccoglie layer e "promuove" alcuni attributi, creando un nodo personalizzato con pochi controlli.
- **JavaScript Shape / Deformer / Utility / Modifier / Emitter, Custom Distribution, SkSL Shader / Filter**: si scrive un nodo nuovo in codice e lo si usa come quelli nativi.

### 1.3 Gli effetti si incastrano tra loro
È questo che rende Cavalry così modulare: le categorie non sono compartimenti stagni.

- **Shape → Shader**: *Shape to Shader* usa una forma come texture di un'altra.
- **Shader → Filter**: *Distortion* e *Distort Edges* deformano i pixel campionando uno shader (per esempio un Noise Shader).
- **Shader → Shader**: *Blend Shader* fonde più shader in uno solo, che si può riusare.
- **Forma → Falloff → Behaviour**: una forma qualsiasi diventa la zona d'influenza di un deformer o di un valore.
- **Distribution → Distribution**: Mask, Sort e Shuffle prendono un'altra distribuzione e la filtrano o riordinano.
- **Duplicator → Duplicator**: l'annidamento con l'indice che si trasmette.
- **Behaviour → attributo di un Behaviour**: i parametri di un modificatore sono a loro volta pilotabili.
- **Filter con matte**: una forma animata limita dove agisce un filtro, con feather.

### 1.4 L'ordine di valutazione (per una forma)
```
Generatore (Basic Shape, Duplicator + Distribution, Text…)
  → Deformers (in lista, ognuno con Strength e Falloff)
  → Materiali: Fill/Stroke con pila di Shader (blend + opacity)
  → Filters (in lista, blend + opacity + mattes)
  → Blend mode della forma sulla composizione
Valori (Behaviour di valore, Utility) entrano a qualsiasi livello tramite collegamenti,
rivalutati per copia grazie al contesto.
```

---

## 2. Come funziona Lumit

Lumit è più simile ad **After Effects** che a Cavalry: è un compositor **a pixel**, a layer, con uno stack di effetti per layer. Ha però aggiunto tre cose che lo avvicinano al procedurale: **driver**, **flussi di punti** e **node graph composition**.

### 2.1 Lo stack di effetti (raster)
- Ogni layer ha uno **stack di effetti** applicato dall'alto in basso. Ogni effetto lavora sull'**immagine** che riceve.
- Ordine per layer: sorgente → retime → masks → **effetti** → transform → blend mode.
- **Ogni effetto ha Mix, Blend (26 modi) e Matte.** Il Matte è un layer qualsiasi, letto in luminanza, alpha o canale, invertibile, e si può prendere grezzo, con le maschere o con i suoi effetti. In alcuni effetti il matte modula il parametro stesso, non solo l'intensità (per esempio il Gaussian blur restringe il raggio).
- **Adjustment layer**: gli effetti agiscono su tutto ciò che sta sotto.

### 2.2 Il Node graph del layer
- È lo **stesso stack** visto come box e fili: Source → effetti → Layer out. Si può bypassare un effetto staccando il filo.
- I **Driver** producono valori (numero, colore, punto) e si collegano ai parametri: Wiggle, Math, Remap, Smooth, Split, Combine, Expression, Colour cycle, Audio level, Beat trigger, Measure, Points sample, Layer points, **Clone index**.
- **Limiti:** in un layer non ci sono rami né merge, e **i fili non attraversano i layer** (i driver che leggono un altro layer lo scelgono da un menu).
- I **gruppi di nodi** si salvano come `.lumgrp` e si reinseriscono dalla ricerca o da una barra (shelf).

### 2.3 Node graph composition (il vero "nodale")
- È una composizione fatta di box: **Read** (footage, solid, comp), **Input** (immagine, numero, angolo, colore), **Effect** (qualsiasi effetto o driver), **Merge** (A su B con blend e opacità), **Switch** (scelta per indice), **Time offset**, **Node graph** annidato e **Output**.
- Qui ci sono rami e merge, un'immagine si può usare più volte e i grafi si annidano.
- **Riuso come effetto:** l'effetto *Node graph* applica un grafo a un layer qualsiasi. I suoi **Input** diventano i controlli dell'effetto. È l'equivalente più vicino a *Component* e *Pre-Comp Overrides* di Cavalry.
- Il grafo si può anche piazzare come layer precomp, e i suoi Input diventano controlli del layer.

### 2.4 Flussi di punti (la parte "Cavalry-like")
Esiste una famiglia di effetti che produce, modifica e consuma **flussi di punti**, collegati tra loro con un tipo di filo dedicato:

- **Produttori:** Grid (Rows, Offset rows, Rings, Spiral, Hexagons, Point), Scatter, Points along path, Text to points, Emit from image, Track points, Particulate.
- **Modificatori:** Vary points, Pick points, Transform points, Relax points, Flow points, Merge points, Bake points.
- **Consumatori:** Clone to points, Connect points, Trail, Label points, Points field.
- **Attributi per punto ("Names"):** ogni punto può portare fino a 64 valori con nome, più quelli integrati: `@index`, `@indexnorm`, `@n`, `@id`, `@x/@y/@z`, `@size`, `@age`, `@rand01`, `@picked`. È l'equivalente dell'indice/contesto di Cavalry, ma *per punto* invece che per copia di un nodo qualsiasi.
- **Vary points:** un **Pattern** (Index, Random, Noise, Distance, Age, Across, Down, Image, Stripes, Checker, Waves, Number…) passato per una **Curve**, che pilota dimensione, opacità, rotazione, offset e colore. È lo Stagger e il Falloff di Cavalry insieme.
- **Clone to points:** timbra fino a 4 layer sui punti, sceglie il layer per ordine, a caso o per numero, e applica offset di tempo per copia. Con il driver **Clone index** dentro il layer clonato, ogni copia viene renderizzata diversa, ma con un **massimo di 32 render unici**.

### 2.5 Altri strumenti
- **Shape layer:** gruppi vettoriali con fill e stroke, gradienti Linear/Radial fino a 16 stop, **Combine** (Union, Subtract, Intersect, Exclude) e operazioni in **ordine fisso**: smooth → twist → deform (noise) → offset → trim → repeater (massimo 100 copie). Poi stroke taper e tratteggio.
- **Masks** con modi (Add, Subtract, Intersect, Difference, Lighten, Darken) e **Mattes** da qualsiasi layer.
- **Espressioni** JavaScript (compatibili con After Effects) o Rhai su proprietà e parametri; **Custom controls** per esporre valori.
- **Preset** `.lumfx` (stack di effetti con keyframe) e **script** `.jsx` / `.js`.
- **Custom shader** (WGSL scritto a mano o costruito con un grafo di box) ed **estensioni LFX**.

---

## 3. Confronto concetto per concetto

| Concetto Cavalry | In Lumit | Stato |
|---|---|---|
| Collegare un attributo a qualsiasi altro | Driver → parametro (stesso layer); Espressioni (anche tra layer) | 🟡 parziale: fili solo dentro il layer, tra layer solo via espressioni o picker |
| Behaviour che pilota parametri di un altro behaviour | Driver → driver (es. Wiggle → Remap → parametro) | ✅ |
| Contesto per indice (per copia) | `@index` e Names sui punti; Clone index nel layer clonato | 🟡 solo dentro la famiglia punti; 32 render unici |
| Duplicator annidati con Depth | — | ❌ non documentato |
| Array per indice (colori, forme, asset) | Clone to points con 4 layer + *Choose by*; colour ramp di Vary points; array in espressione | 🟡 |
| Falloff (forma, curva, layer mode) | Matte su ogni effetto; Pattern + Curve in Vary points; Pick points | 🟡 il matte è un'immagine, non una pila di falloff con modi |
| Behaviour Mixer | Catena di Math / Expression | 🟡 manuale |
| Lista Deformers sulla forma | Operazioni shape in ordine fisso | ❌ niente pila libera di deformer geometrici |
| Shader stack nei materiali | Gradient del fill (2 tipi) + effetti Generate + Merge nel node graph | 🟡 |
| Filter stack con blend + matte | Stack effetti con Mix, Blend, Matte | ✅ (anzi più flessibile) |
| Shape to Shader / Shader → Distortion | Displacement map, Texturize, Custom shader con *Second input*; nel node graph: qualsiasi ramo | ✅ nel node graph |
| Blend Shader | Box **Merge** nel node graph | ✅ |
| Pre-Comp Overrides / Component | Node graph con **Input** + effetto *Node graph*; Custom controls | ✅ concetto presente, solo per effetti/immagini |
| Distribution | Grid / Scatter / Points along path / Text to points / Emit from image… | 🟡 mancano Fibonacci, Rose, Math, Intersections, Voxelize, Sort/Shuffle, Shape edges come tali |
| Duplicator | Clone to points | 🟡 |
| Particelle | Particulate + Flow points | 🟡 meno modificatori |
| Forge Dynamics (fisica) | — | ❌ |
| JavaScript Shape / Deformer / Distribution | — (Expression per i valori, Custom shader per i pixel) | ❌ geometria via codice assente |
| SkSL Shader / Filter | Custom shader WGSL / LFX | ✅ |
| Value Solver, Spring, Trails | Smooth driver, Echo, Trail (sui punti) | 🟡 |
| Spreadsheet, String Generator / Manipulator | Espressioni Rhai sul testo | 🟡 / ❌ |

### Gli effetti mancanti più importanti
- **Shader:** gradiente **conico, sweep e multi-point**, **Voronoi**, **Checkerboard**, SLA noise (c'è solo Fractal noise).
- **Filtri:** **Gradient Map** a N stop (c'è Tritone a 3), **Halftone**, **Dithering**, **Inner Shadow**, **Erosion** (choke/dilate), **Light Sweep**, **Lighting** (ombre lunghe), **Bilateral blur**, **Scrape**, **Stripes**, **Slit Scan**.
- **Deformer geometrici:** Bend, Lattice, Four Point Warp, Wave sul path, Bevel, Subdivide, Path Offset in pila, Boolean dinamico tra layer, Morph tra forme diverse, Pathfinder, Squash & Stretch, Motion Stretch.

---

## 4. Cosa si può fare in LFX

### 4.1 Cosa offre LFX (dal codice, `api: 1`)
- Una cartella o un pacchetto `.lfx` con `extension.json` e uno shader **WGSL**, opzionalmente con una pagina HTML d'interfaccia.
- Fino a **8 passaggi** (pass) per effetto, anche a ½ o ¼ di risoluzione; ogni pass può leggere i precedenti.
- Legge: l'**input** dell'effetto, il **layer originale** (prima degli effetti), **un secondo input** (un altro layer o una mappa di profondità), le dimensioni, `time` e `seed`.
- Parametri da `struct Params`: slider, bounded, angolo, intero, toggle, menu (choice), seed, colore, punto. **Sono parametri normali**: la doc del Custom shader, di cui LFX usa lo stesso contratto, dice che hanno keyframe ed espressioni come quelli nativi. Quindi dovrebbero accettare anche i fili dei **driver** nel node graph (da verificare in Lumit).
- In uscita: **un'immagine**. Più Mix, Blend e Matte, che Lumit aggiunge da solo.

### 4.2 Il trucco per avere la modularità di Cavalry dentro Lumit
Visto che i parametri LFX accettano driver, un effetto LFX diventa "Cavalry-like" se lo si combina con gli altri pezzi:
- **Per copia:** si mette l'effetto LFX nel layer clonato e si collega **Clone index → Normalised / Random** a un suo parametro. Ogni copia di Clone to points avrà un gradiente o un colore diverso, come con Stagger + Duplicator.
- **Falloff:** un LFX che **genera una maschera** (cerchio, box, lineare, sweep, con curva e ripetizioni) da usare come **Matte** di qualsiasi altro effetto. È il Falloff di Cavalry tradotto in pixel.
- **Blend Shader / Shape to Shader:** nel node graph si combinano più box LFX con **Merge**, poi si salva il gruppo (`.lumngrp`) o si applica tutto come effetto *Node graph* con gli Input esposti.

### 4.3 Fattibilità effetto per effetto

| Effetto Cavalry | In LFX? | Note |
|---|---|---|
| Gradient conico / sweep / multi-point | ✅ facile | 1 pass; numero di stop fisso (es. 4–6 colori come parametri) |
| Voronoi, Checkerboard, SLA / altri noise | ✅ facile | 1 pass, animabili con `time` |
| **Falloff** come generatore di matte | ✅ facile | Circle, Box, Linear, Sweep, Probability; curva approssimata con ease (non esiste un parametro "curva") |
| Gradient Map a N stop | ✅ | stop come parametri colore |
| Halftone, Dithering, Stripes, Scrape | ✅ | 1–2 pass |
| Inner Shadow, Erosion / Dilate | ✅ | 2–3 pass (sfocatura separabile + soglia) |
| Light Sweep, Lighting (ombre lunghe) | ✅ / 🟡 | ombre lunghe con ricerca a raggi su più pass |
| Bilateral blur | ✅ | 2 pass |
| Bend, Wave, Four Point Warp, Lattice semplice | 🟡 | **solo come distorsione dell'immagine**, non della geometria |
| Distortion che campiona uno shader | ✅ | shader di rumore come *second input*, oppure rumore interno |
| Slit Scan, Trails, Value Solver | ❌ | LFX non vede altri frame (solo `time`); servono Echo o Time offset nel node graph |
| Deformer geometrici veri, Boolean, Bevel, Subdivide | ❌ | LFX produce pixel, non path |
| Distribution nuove (Fibonacci, Rose, Math…) | ❌ | LFX non produce flussi di punti |
| Behaviour di valore (Spring, Random, Stagger come nodo) | ❌ | LFX non produce numeri; si usano driver ed espressioni |
| Fisica, testo procedurale, spreadsheet | ❌ | fuori dal modello LFX |

### 4.4 Quello che non si fa in LFX si può fare comunque
Lumit è open source (Rust), quindi le parti marcate ❌ si possono aggiungere **nel motore**:
- nuove **distribuzioni di punti** (Fibonacci, Rose, Math): sono effetti della famiglia punti in Rust;
- nuovi **driver** (Spring, Stagger come nodo, Array per indice);
- **deformer geometrici** per gli shape layer, cioè una pila libera di operazioni al posto dell'ordine fisso.

Sono contributi al codice di Lumit (pull request), non estensioni.

---

## 5. Note e verifiche aperte
- Nella documentazione di Lumit, la pagina *Effects* dice ancora "LFX — Not built yet", mentre la pagina *Writing LFX effects* e il codice (`crates/lumit-core/src/fx/lfx.rs`) lo implementano. Va controllato se la versione installata lo supporta già (Settings ▸ Addons ▸ Extensions).
- La divisione "deformer / valore" dei Behaviour di Cavalry è una mia classificazione.
- Il limite di 32 render unici di Clone to points e il massimo di 100 copie del repeater sono scritti nella documentazione di Lumit.
