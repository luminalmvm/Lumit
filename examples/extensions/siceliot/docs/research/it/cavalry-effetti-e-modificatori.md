# Cavalry — effetti e modificatori

*English: [../en/cavalry-nodes-by-role.md](../en/cavalry-nodes-by-role.md)*

> **Note di ricerca del 26 ottobre 2026.** Per sapere quali di questi copre ora SICELIOT, vedi
> [../../SICELIOT.md](../../SICELIOT.md).

Elenco completo dei nodi documentati su [cavalry.studio/docs](https://cavalry.studio/docs/) (sitemap letta il 26/10/2026), raggruppati per **ruolo nella catena non distruttiva**. Le descrizioni sono riassunte dalla documentazione ufficiale.

> La divisione dei Behaviour in *Deformer* e *Valore* è una mia lettura di cosa produce ogni nodo; la documentazione li elenca tutti insieme come "Behaviours".

## Come si incastrano (in breve)

```
 Utilities (math, array, logic, context, falloff)
        │ numeri / colori / stringhe
        ▼
 Behaviour "valore" ──collegamento──▶ qualsiasi attributo (anche di altri behaviour)
                                         │
 Distribution ──▶ Duplicator / Connect / Points to Path …   (punti → geometria)
                                         │
 Shape (generatore) ──▶ lista Deformers ──▶ Materiali (Fill/Stroke + pila Shader) ──▶ pila Filter ──▶ Blend mode della shape
                          ▲ ognuno con Falloff                 ▲ blend + opacity       ▲ blend + opacity + matte
```

- **Collegamenti**: un attributo può pilotarne molti (output), ma ha un solo input; i keyframe contano come input.
- **Context / indice**: il Duplicator chiede il valore *una volta per copia*, passando l'indice, quindi lo stesso Random/Stagger/Noise dà un valore diverso a ogni copia; l'indice attraversa Duplicator annidati.
- **Pile ordinate con blend**: Falloff (Add/Min/Max/Minus/Multiply/Screen/Overlay), Behaviour Mixer (stessi modi), Shader (blend mode + opacity), Filter (blend mode + opacity + matte).
- **Ordine fisso**: gli Shader vengono applicati ai materiali *prima* dei Filter.

---

## 1. Shape — generatori di geometria

### Primitive
| Nodo | Cosa fa |
|---|---|
| Basic Shape | Primitiva: Rectangle, Ellipse, Polygon, Star, Arc, Arrow, Capsule, Cogwheel, Ring, Super Ellipse, Super Shape |
| Basic Line | Linea: Line, Bézier, Spiral |
| Background Shape | Forma che si adatta alla risoluzione della composizione |
| Editable Shape | Forma con punti modificabili a mano |
| Text Shape | Testo |
| SVG | Forme da file .svg |
| Footage Shape | Forma con le dimensioni del footage |
| Cel Animation Shape | Animazione frame per frame |
| Spacer | Mesh vuota per i layout |

### Procedurali (prendono altre forme o punti in input)
| Nodo | Cosa fa |
|---|---|
| **Duplicator** | Duplica forme sui punti di una Distribution |
| Custom Shape | Istanza che eredita un'altra forma |
| Connect Shape | Collega punti di distribuzioni con linee |
| Points to Path | Crea un path dai punti di una distribuzione |
| Outline | Contorno geometrico dal path di un'altra forma |
| Extrude | Estrusione per dare profondità |
| Merge | Unisce due path aperti in un path chiuso |
| Convex Hull | Forma che racchiude altre forme |
| Metaballs Shape | Fonde più forme in un blob organico |
| Trails | Scie dal movimento di altre forme |
| Ray | Raggio infinito che si ferma sulle collisioni |
| Shortest Path | Percorso più breve tra punti di una distribuzione |
| Quad Tree Shape | Suddivide un quadrato in base a una distribuzione |
| Segment Path | Spezza un path in segmenti (sub-mesh) |
| Extract Sub-Meshes | Estrae sotto-mesh specifiche da un'altra forma |
| Sub-Mesh Bounding Box | Rettangoli dai bounding box delle sotto-mesh |
| Image to Shapes | Forme campionando un'immagine |
| Isolines Shape | Contorni (isolinee) da immagini o forme |
| Mesh Shape | Mesh con vertici per deformare immagini o shader |
| Corner Pin | Prospettiva finta a 4 angoli |
| Rectangle Pattern | Grafici a barre e a torta |
| Particle Shape | Sistema particellare (fumo, pioggia, polvere) |
| Forge Dynamics Shape | Simulazione fisica nativa (con Fields, Constraints, Collision Events) |
| JavaScript Shape | Forma scritta in JavaScript |

### Contenitori e composizione
| Nodo | Cosa fa |
|---|---|
| Group | Raggruppa e dà una trasformazione comune |
| Component | Contenitore con attributi "promossi", come un preset |
| Composition | Lo stage; usabile come pre-comp |
| Pre-Comp Overrides | Sovrascrive attributi dentro una pre-comp (template) |
| Layout Group / Grid Layout Group / Grid Layout Row | Impila forme in righe, colonne e griglie |
| Layout Shape | (Deprecato) vecchio layout |

---

## 2. Distribution — sorgenti di punti

Si collegano al Duplicator e ad altri nodi che lavorano su punti, e possono anche essere ri-distribuite.

| Nodo | Cosa fa |
|---|---|
| Point | Tutti i punti in 0,0 |
| Linear | In linea orizzontale o verticale |
| Grid | Griglia |
| Circle | Cerchio |
| Fibonacci | Spirale di Fibonacci |
| Rose | Curva a rosa |
| Random | Posizioni casuali |
| Path | Lungo uno o più path |
| Shape Edges | Sui bordi di un'altra forma |
| Shape Points | Sui vertici di un'altra forma |
| Sub-Mesh | Sulle posizioni delle sotto-mesh |
| Transform | Sulla posizione di un layer |
| Intersections | Dove le forme si intersecano |
| Mask | Elimina i punti fuori da una forma |
| Voxelize | Voxelizzando un'altra forma |
| Particle | Sulle particelle di una Particle Shape |
| Array | Una posizione per ogni indice |
| Math | Espressioni matematiche |
| Custom | JavaScript |
| Shuffle | Rimescola gli id |
| Sort | Riordina gli id di un'altra distribuzione |

---

## 3. Behaviour — DEFORMER (modificano la geometria, vanno nella lista *Deformers*)

### Deformazione
| Nodo | Cosa fa |
|---|---|
| Noise | Deforma con rumore (o genera valori) |
| Oscillator | Onde trigonometriche (deforma o genera valori) |
| Wave | Onde cicliche lungo path aperti e chiusi |
| Bend Deformer | Piega attorno a un cerchio |
| Lattice Deformer | Gabbia di punti di controllo |
| Four Point Warp | Warp a 4 angoli con maniglie bézier |
| Pinch | Pizzica/tira dentro un falloff muovendo un Null |
| Push Along Vector | Spinge lungo un vettore |
| Flare | Rastrema lungo un asse |
| Skew | Inclinazione |
| Squash and Stretch | Schiaccia e allunga (bulge, area preservata) |
| Motion Stretch | Smear in base al movimento |
| 3D Matrix | Trasformazioni 3D finte su forme e sotto-mesh |
| Manipulator | Applica trasformazioni alle forme |
| Look At | Ruota verso un bersaglio |
| Align | Pivot dinamici sul bounding box |
| Pathfinder | Muove o deforma lungo un path |
| Morph | Morph tra forme con numero di punti diverso |
| Blend Shape | Blend tra forme con stesso numero di punti |
| Blend Sub-Mesh Positions | Blend tra posizioni di sotto-mesh di due forme |
| JavaScript Deformer | Deformer scritto in JavaScript |
| Mesh Solver | Concatena deformer in modo iterativo (crescita differenziale, macchine da disegno) |

### Operazioni sui path (topologia)
| Nodo | Cosa fa |
|---|---|
| Boolean | Unione/sottrazione procedurale |
| Path Offset | Allarga/restringe il path |
| Bevel | Angoli arrotondati o smussati |
| Subdivide | Suddivide e ammorbidisce |
| Add Divisions | Aggiunge punti per deformazioni più morbide |
| Resample Path | Semplifica / rimuove sovrapposizioni |
| Clean Up | Rimuove punti inutili |
| Curves to Lines | Bézier → segmenti |
| Path Average | Ammorbidisce mediando i punti |
| Path Relax | Allontana i punti tra loro |
| Reverse Path | Inverte la direzione |
| Travel Deformer | Sposta il punto di partenza dei contorni |
| Extend Open Paths | Allunga le estremità dei path aperti |
| Chop Path | Taglia in fette |
| Knot | Spazi dove il contorno si auto-interseca |
| Fill Rule | Come buchi e auto-intersezioni influenzano il fill |
| Stitches | Riempie con effetto cucitura |
| Voxelize | Converte in voxel |
| Flatten Shape Layers | Appiattisce la gerarchia interna in un path |
| Auto-Crop | Ritaglia sul bounding box |

### Sotto-mesh (lavorano sui "pezzi" di Duplicator, testo, SVG…)
| Nodo | Cosa fa |
|---|---|
| Sub-Mesh | Accede alle sotto-mesh per trasformazioni, materiali, ecc. |
| Contours to Sub-Meshes | Contorni → sotto-mesh |
| Apply Distribution | Mette le sotto-mesh su una distribuzione |
| Apply Layout (Grid / Horizontal / Vertical) | Impila le sotto-mesh |
| Visibility Sequence | Visibilità delle sotto-mesh in sequenza |
| Color Material Override | Sovrascrive i colori delle sotto-mesh |
| HSV Material Override | Sovrascrive H/S/V delle sotto-mesh |
| Alpha Material Override | Sovrascrive l'alpha delle sotto-mesh |
| Swap Color Override | Cerca e sostituisce colori specifici |

### Animazione automatica
| Nodo | Cosa fa |
|---|---|
| Auto-Animate | Animazioni di trasformazione/opacità con pochi keyframe |
| Spring | Moto secondario elastico sui keyframe di posizione |
| Rubber Hose Limb | IK a 2 ossa per arti |

---

## 4. Behaviour — VALORE (generano numeri o colori da collegare a qualsiasi attributo)

| Nodo | Cosa fa |
|---|---|
| **Stagger** | Valori sequenziali tra min e max, per indice (con grafico) |
| Random | Numeri casuali con bias |
| Noise / Oscillator / Sound | Anche come sorgenti di valori |
| Value / Value2 / Value3 | Memorizzano e manipolano valori 1D/2D/3D |
| Value Blend / Value2 Blend / Value3 Blend | Interpolano (lerp) tra due valori |
| Position Blend | Blend tra due posizioni |
| Color Blend | Blend tra due colori con un gradiente |
| Number Range | Rimappa un intervallo in un altro |
| Number Range to Color | Rimappa valori su un gradiente di colori |
| Area Range | Interpreta valori come area e li rimappa in dimensioni |
| Round | Arrotonda / a multipli |
| Modulate | Sequenze ripetute (modulo) |
| Frame | Usa il tempo per pilotare attributi |
| Distance | Pilota un attributo con la distanza da un altro layer |
| Get Vector | Vettore tra due punti |
| Is Within | Vero se una forma è dentro un'altra |
| Material Sampler | Campiona il colore di una forma |
| Value Solver / Value2 Solver | Accumulano o sfumano valori nel tempo (memoria dei frame precedenti) |
| **Behaviour Mixer** | Combina più behaviour di valore in pila con blend mode (non accetta deformer) |

---

## 5. Shader — riempimenti procedurali (pila nei materiali Fill/Stroke)

| Nodo | Cosa fa |
|---|---|
| Color Shader | Colore singolo |
| Gradient Shader | Gradienti; il tipo è un generatore interno (Linear, Radial, Conical e altri) |
| Multi-Point Gradient Shader | Gradiente a più punti con influenza |
| Noise Shader | Rumore Perlin, Value o Cellular |
| SLA Shader | Rumori "nuovi" |
| Voronoi Shader | Pattern cellulare |
| Checkerboard Shader | Scacchiera |
| Image Shader | Immagini, sequenze, video |
| **Shape to Shader** | Usa una forma come shader per un'altra forma |
| **Blend Shader** | Combina più shader in uno solo |
| SkSL Shader | Shader scritto in SkSL (codice) |

Attributi comuni: **Blend Mode** (come si fonde con gli shader sotto nella pila) e **Opacity**.

---

## 6. Filter — effetti raster (pila sulla shape)

### Sfocature
Gaussian Blur · Fast Blur · Box Blur · Bilateral Blur · Directional Blur · Zoom Blur · Luminance Blur (sfoca in base alla luminanza di un altro layer) · Background Blur (sfoca ciò che sta dentro un'altra forma)

### Colore e tono
Brightness and Contrast · Levels · Curves · Gamma Correction · HSV Adjustment · Black and White · Invert · Threshold · Posterize · TriTone · **Gradient Map** · Fill Color · Shift Channels · Dithering · Chroma Key

### Distorsione
**Distortion** (distorce campionando uno *shader*) · **Distort Edges** (bordi distorti tramite shader) · Ripple · Twist · Bulge · Spherise · Polar Coordinates · Mirror · Scrape · Stripes · Slit Scan · Pixelate · Pixel Sorting · RGB Split · Chromatic Aberration

### Stile e luce
Glow · Drop Shadow · Inner Shadow · Lighting (luci puntiformi, ombre lunghe) · Light Sweep · Halftone · Grain · Scan Lines · Vignette · Edge Detection · Erosion · Sharpen

### Transizioni
Linear Wipe · Radial Wipe · Venetian Blinds

### Codice
SkSL Filter

Attributi comuni: **Blend Mode**, **Opacity**, **Mattes** (lista di forme che limitano il filtro, ognuna con modalità luminance/alpha, blend, opacità, feather, e Apply Mode *Filter Input* o *Clip Output*).

Motion Blur è un'impostazione generale, non un filtro della lista.

---

## 7. Utility — mattoni per costruire logica

### Controllo dell'effetto
| Nodo | Cosa fa |
|---|---|
| **Falloff** | Attenua behaviour e deformer nello spazio (pila con blend mode) |
| Range Falloff | Attenua per intervallo di valori |
| **Index Context** | Fornisce l'indice ad altri layer |
| Length Context | Lunghezza del path come valore per copia |
| Velocity Context / Velocity Magnitude Context | Direzione/velocità di ogni copia nel Duplicator |
| Layer Seed | Seed casuali unici per connessione |
| Accumulator | Impila forme o crea layout dentro un Duplicator |

### Matematica e logica
Math · Math2 · Math3 · Jsmath · Comparison · Logic · If Else · Sequence (sequenze non ripetute) · Seconds to Frames · Local Time · Timeline Counter (accumula o fa scattare valori con i marker) · Animation Control (rimappa una curva di animazione in percentuale)

### Array (assegnazione per indice)
Value Array · Value2 Array · Value3 Array · Color Array · Shader Array · Shape Array · Asset Array · String Array · Typeface Array · Array Manipulator (sperimentale)

### Colore
HSV Color · Color Info · Contrasting Color · Index to Color

### Misure e vincoli
Measure · Measure Text · Bounding Box · Path Length · Radius · Count Sub-Meshes · Get Sub-Mesh Transform · Get Name · Transform Constraint · Bounding Box Constraint · Composition Constraint · Component Constraint

### Materiali condivisi
Fill · Stroke · Stroke Duplicator (più stroke combinati)

### Campionamento e dati
Image Sampler · Spreadsheet · Spreadsheet Lookup · Asset from Smart Folder · String From Asset

### Controlli e rig
Null · Rig Control · Custom Dropdown · Camera · Camera Guide · Lattice Controller · Scheduling Group (posiziona i layer figli nel tempo)

### JavaScript
JavaScript Utility · JavaScript Modifier · JavaScript Emitter · (più JavaScript Shape, JavaScript Deformer e Custom Distribution nelle altre categorie)

### Particelle — emettitori e modificatori
Particle Emitter · Distribution Emitter · Force Modifier · Turbulence Modifier · Flow Field Modifier · Vortex Modifier · Magnetic Modifier · Goal Modifier · Path Modifier · Speed Modifier · Data Modifier · Visual Modifier · Image Modifier

### Forge Dynamics — campi, vincoli, eventi
Attractor Field · Direction Field · Drag Field · Vortex Field · Buoyancy Field · Path Field · Distance Constraint · Pin Constraint · Bridge Constraint · Color / Visibility / Impulse / Sticky / Body Settings Collision Event

### Testo
String · String Generator (Value, Random Number, Random Date, Formatted String, Formatted Date/Time, Hash, Hexadecimal, Timecode, Block) · String Manipulator (Change Case, Join, Regex, Replace, Resize, Shuffle, Sub-String, Transition, Unicode Offset) · String Length · Typeface · Apply Typeface / Font Size / Font Style / OpenType / Character Spacing / Text Material

---

## Riferimenti utili nella docs
- Concetti chiave: [Connections](https://cavalry.studio/docs/getting-started/key-concepts/connections/), [Context](https://cavalry.studio/docs/getting-started/key-concepts/context/), [Layers](https://cavalry.studio/docs/getting-started/key-concepts/layers/)
- Pile e blend: [Common Attributes (Behaviours)](https://cavalry.studio/docs/nodes/behaviours/common-attributes-behaviours/) (falloff), [Behaviour Mixer](https://cavalry.studio/docs/nodes/behaviours/behaviour-mixer/), [Common Attributes (Shaders)](https://cavalry.studio/docs/nodes/effects/shaders/common-attributes-shaders/), [Common Attributes (Filters)](https://cavalry.studio/docs/nodes/effects/filters/common-attributes-filters/), [Blend Modes](https://cavalry.studio/docs/nodes/general/blend-modes/)
