# Progetti di esempio

Due progetti in `examples\`: [Flood](#flood) (1080 × 1920, 30 fps, 20 secondi), da una scena
condivisa su [Scenery](https://scenery.io/scenes), e [Sketch](#sketch) (1920 × 1080, 60 fps,
5 secondi), da uno sketch Processing. Tutto resta modificabile: nessuna impostazione è "cotta" nei
pixel, e ogni parte che si muove è un keyframe o un driver collegato a una riga, che puoi regolare,
ricollegare o spegnere.

*English: [../en/examples.md](../en/examples.md)*

## Aprirli

1. Usa il Lumit compilato da questo repository (`D:\Siceliot\Lumit\…\Release\lumit.exe`), non
   quello ufficiale: Type fill, i driver e le righe di Label points che usano sono modifiche
   all'engine.
2. **File ▸ Open project…** `examples\flood.lum` (comp *Flood*) o `examples\sketch.lum` (comp
   *Copies*).
3. I driver sono nel **Node graph** di ogni layer (`Window ▸ Node graph`, con il layer
   selezionato).

`examples\make-flood.ps1` ricrea Flood da zero, e `examples\render.js` renderizza una comp con
`lumit-cli` (`--arg comp=… --arg from=… --arg to=… --arg dir=…`).

## Flood

Da *Submesh Boolean* di Tim Siegert (Cavalry, [Scenery](https://scenery.io/scenes),
CC BY-NC 4.0): righe di "Flood the Grid" riempiono un quadro 1080 × 1920, e delle forme vi
entrano ed escono. Fuori dalle forme le parole sono nere su bianco, dentro bianche su nero, e da
entrambe le parti si allungano per riempire esattamente lo spazio che hanno. 30 fps, 20 secondi;
lo costruisce `examples\make-flood.ps1`.

![Flood](../images/example-flood.png)

- **Shapes** (una precomp): solidi bianchi ritagliati da **Falloff** (Output Alpha, Softness 0),
  ognuno acceso per quattro secondi: un rombo che si apre e si chiude (0–4 s), un cerchio che
  sale (4–8 s), una G gigante che cresce e ruota (Label points su un solo punto di Grid, 8–12 s),
  due colonne che si separano (12–16 s) e un'ellisse alta (16–20 s). Dimensioni e posizioni sono
  keyframe con easing.
- **Outside**: **Type fill** con il livello Shapes come **Region layer**, **Invert region** e
  **Cut words** attivi (le forme tagliano le parole, non le ripetono), aspetto dal Text layer nascosto *Look black* (Impact, nero).
- **Shapes** nella comp principale, riempito di nero con Fill: l'interno.
- **Inside**: un secondo Type fill sulla stessa regione, non invertito, Cut words attivo, aspetto da *Look white*.
- **Paper**: un solido bianco.

Riscrivi **Words**, cambia il font dei Text layer o aggiungi forme alla precomp: le parole
trovano da sole il nuovo spazio. **Rows**, **Row gap**, **Word gap** e **Min width** regolano la
densità.

Solo uso non commerciale, come chiede la licenza dell'originale.

## Sketch

Dallo sketch Processing "lettere su lettere": una lettera gigante piena di piccoli caratteri,
attraversata da anelli che li colorano, ingrandiscono, ruotano e cambiano; un carattere lasciato
da un anello resta cambiato.

![Sketch](../images/example-sketch.png)

**Come è costruito.** I passi qui sotto lo ricostruiscono in qualsiasi composizione (quella
dell'esempio è 1920 × 1080, 60 fps). `tests\copies\wire-sketch.ps1` fa lo stesso da script, e con
`GRADIENT=1` usa Flash colour *Gradient*.

1. Crea due layer di testo e nascondili entrambi. **Mask** contiene una lettera grande in grassetto
   (Arial Bold, 720 px) e decide quali punti restano. **Look** contiene un carattere qualsiasi
   (Arial, 12 px, bianco) e dà ai caratteri il loro aspetto.
2. Su un layer **Points**, aggiungi **Grid** con 75 × 75 punti, Spacing 12 px e Mix 0 (niente
   dischi). Imposta Position x e Position y con le espressioni `comp_width / 2` e `comp_height / 2`
   (clic destro sul valore, **Set expression**), così la griglia resta al centro della composizione
   a qualunque dimensione.
3. Sotto, aggiungi **Pick points** con Pattern *Image*, Image layer Mask, Map placement Centre,
   From 1 e Mix 0, e collegaci i Points di Grid. Restano solo i punti sulla lettera, compreso il
   bordo sfumato.
4. Sotto ancora, aggiungi **Label points**, con i Points collegati da Pick points. Imposta Text layer
   su Look e Text su `{pick:A|B|C|…|&|*}`, con Anchor Centre, **Vertical anchor** *Top*,
   Offset x 0, Max labels 2000, Change above 80, Scramble rate 60 e Flash colour *By strength*.
5. Nel Node graph, aggiungi un **Ring wave**: Origin *Away from points*, Clearance 9, **Add up**
   attivo, Interval 0,5 s, Speed 300, Thickness 60–120, Grow 20–50. Collega **Value** a Change e
   **Passes** a Text step.
6. Per la dimensione, aggiungi una **Math** (Add) con A = 100 e **Grow** del Ring wave in B. Il suo
   Value va su Label scale.
7. Per la rotazione, aggiungi un **Value noise** (Vary by *Position*, Noise scale 100, Frequency
   0.2, Detail 4, Minimum −18, Maximum 18). Mandalo, insieme a **Rotation** del Ring wave, in una
   seconda **Math** (Add), il cui Value va su Label rotation.

Dove passa il centro di un anello (Change sopra 80), il carattere cambia a ogni fotogramma, poi si
ferma su uno nuovo che resta. Tutto intorno le lettere si ingrandiscono, ruotano e prendono colore,
e quando l'anello è passato tornano bianche e ferme. Gli anelli nascono fuori dalla lettera e la
attraversano.

- **Un'altra dimensione.** Per una lettera più grande, alza anche Columns e Rows di Grid, così la
  griglia la copre.
- **Un altro layer.** Copia gli effetti del layer Points in Effect Controls e incollali su un altro
  layer. Arrivano anche il Ring wave, il Value noise, le due Math e tutti i collegamenti. Mask e Look
  sono layer, non effetti, quindi lasciali nella composizione.

Ogni riga è spiegata in [effetti.md](effetti.md#label-points).

## Effetti usati

| Effetto | Flood | Sketch |
|---|---|---|
| Type fill | ✓ | |
| Falloff | ✓ | |
| Grid | ✓ | ✓ |
| Pick points | | ✓ |
| Label points | ✓ | ✓ (By strength, Text step, Grow, Label rotation, Vertical anchor) |
| Ring wave | | ✓ |
| Value noise | | ✓ |
