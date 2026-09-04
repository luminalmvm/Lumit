// Lumit Bridge fixture builder.
//
// In plain terms: this script builds one small After Effects project that uses
// every feature the importer has to carry - nested comps, the whole variety of
// keyframe, both kinds of matte, masks, markers, retiming, text, shapes, a
// camera and a light, expressions, and a spread of effects including the ones
// that cannot be read - saves it as fixture.aep, then runs the walker over it.
// The bundle that comes out is the repo's golden fixture: the Rust importer's
// tests read it, so the feature list here is the coverage checklist in
// docs/impl/ae-import.md section 5, row for row.
//
// It is deterministic on purpose: every value is written, nothing is sampled
// from the clock, and no font is chosen (AE's default is whatever the machine
// has). Only the manifest's export date changes between runs.
//
// How to run (After Effects 2024+):
//   Save and close whatever you are working on first - this builds a NEW project.
//   Edit > Preferences > Scripting & Expressions > "Allow Scripts to Write Files
//   and Access Network" must be on.
//   File > Scripts > Run Script File...  ->  pick this file
// It writes tools/ae-bridge/fixtures/fixture.aep and
// tools/ae-bridge/fixtures/fixture.lum-bundle/.

(function makeLumitFixture() {
    var scriptFile = new File($.fileName);
    var toolFolder = scriptFile.parent;
    var fixtures = new Folder(toolFolder.fsName + "/fixtures");

    // Every feature is one step. A step that fails is recorded and the build
    // carries on, so a match-name drift in one row costs one row rather than
    // the whole fixture - and the alert at the end names what is missing.
    var problems = [];
    function step(what, fn) {
        try { fn(); } catch (err) { problems.push(what + " - " + String(err)); }
    }
    function shape(verts) {
        var s = new Shape();
        var zeros = [];
        for (var i = 0; i < verts.length; i++) zeros.push([0, 0]);
        s.vertices = verts;
        s.inTangents = zeros;
        s.outTangents = zeros;
        s.closed = true;
        return s;
    }
    function transform(layer) { return layer.property("ADBE Transform Group"); }

    if (!confirm("Build the Lumit fixture project?\n\n" +
        "This creates a NEW After Effects project - save anything open first.")) {
        return;
    }
    if (!app.newProject()) return;

    app.beginUndoGroup("Lumit fixture");

    // --- the item tree ----------------------------------------------------
    var folder = app.project.items.addFolder("Fixture folder");
    var inner = app.project.items.addComp("Fixture inner", 320, 240, 1, 4, 25);
    inner.parentFolder = folder;

    var innerBase = inner.layers.addSolid([0.2, 0.4, 0.8], "inner base", 320, 240, 1, 4);
    step("inner opacity keys", function () {
        var op = transform(innerBase).property("ADBE Opacity");
        op.setValueAtTime(0, 100);
        op.setValueAtTime(2, 25);
        op.setValueAtTime(4, 100);
    });
    step("inner comp marker", function () {
        var mv = new MarkerValue("inner marker");
        mv.duration = 0.5;
        inner.markerProperty.setValueAtTime(1, mv);
    });

    var outer = app.project.items.addComp("Fixture", 640, 360, 1, 10, 25);
    step("comp settings", function () {
        outer.motionBlur = true;
        outer.shutterAngle = 200;
        outer.shutterPhase = -100;
        outer.motionBlurSamplesPerFrame = 24;
        outer.motionBlurAdaptiveSampleLimit = 64;
        outer.bgColor = [0.05, 0.05, 0.06];
        outer.preserveNestedFrameRate = true;
    });

    // --- layers, bottom of the stack first (each new layer lands on top) ---
    var bg = outer.layers.addSolid([0.5, 0.5, 0.5], "bg", 640, 360, 1, 10);
    step("label colour", function () { bg.label = 1; });

    var blendMultiply = outer.layers.addSolid([0.9, 0.3, 0.3], "blend multiply", 320, 200, 1, 10);
    var blendScreen = outer.layers.addSolid([0.3, 0.9, 0.3], "blend screen", 320, 200, 1, 10);
    var blendDissolve = outer.layers.addSolid([0.3, 0.3, 0.9], "blend dissolve", 320, 200, 1, 10);
    var blendOverlay = outer.layers.addSolid([0.9, 0.9, 0.3], "blend overlay", 320, 200, 1, 10);
    step("blend modes", function () {
        blendMultiply.blendingMode = BlendingMode.MULTIPLY;
        blendScreen.blendingMode = BlendingMode.SCREEN;
        blendDissolve.blendingMode = BlendingMode.DISSOLVE;
        blendOverlay.blendingMode = BlendingMode.OVERLAY;
        blendScreen.preserveTransparency = true;
    });
    step("rotation and opacity keys", function () {
        var rot = transform(blendMultiply).property("ADBE Rotate Z");
        rot.setValueAtTime(0, 0);
        rot.setValueAtTime(2, 180);
        var op = transform(blendMultiply).property("ADBE Opacity");
        op.setValueAtTime(0, 100);
        op.setValueAtTime(2, 40);
    });

    // Legacy matte: the matte layer sits directly above its consumer.
    var lumaMatted = outer.layers.addSolid([0.8, 0.6, 0.2], "luma matted", 400, 300, 1, 10);
    var lumaMatteSource = outer.layers.addSolid([0.7, 0.7, 0.7], "luma matte source", 400, 300, 1, 10);
    step("legacy luma matte", function () {
        lumaMatted.trackMatteType = TrackMatteType.LUMA;
    });

    // Modern matte: names its matte layer, which need not be adjacent.
    var alphaMatted = outer.layers.addSolid([0.2, 0.8, 0.8], "alpha matted", 400, 300, 1, 10);

    var rigNull = outer.layers.addNull(10);
    rigNull.name = "rig null";

    var childA = outer.layers.addSolid([0.9, 0.5, 0.1], "child A", 200, 200, 1, 10);
    step("parenting chain (child A)", function () { childA.parent = rigNull; });
    step("position keys: ease, hold, roving, spatial tangents", function () {
        var pos = transform(childA).property("ADBE Position");
        pos.setValueAtTime(0, [100, 100]);
        pos.setValueAtTime(1, [200, 160]);
        pos.setValueAtTime(2, [320, 120]);
        pos.setValueAtTime(3, [440, 240]);
        pos.setValueAtTime(4, [560, 180]);
        pos.setInterpolationTypeAtKey(4, KeyframeInterpolationType.BEZIER,
            KeyframeInterpolationType.HOLD);
        // A spatial property takes exactly one ease object per side.
        pos.setTemporalEaseAtKey(1, [new KeyframeEase(0, 20)], [new KeyframeEase(0, 75)]);
        pos.setTemporalEaseAtKey(5, [new KeyframeEase(0, 75)], [new KeyframeEase(0, 20)]);
        pos.setSpatialTangentsAtKey(3, [-40, -10], [40, 10]);
        // Roving last: setting interpolation or ease on a key clears it.
        pos.setRovingAtKey(2, true);
    });
    step("enabled expression", function () {
        transform(childA).property("ADBE Opacity").expression = "50 + 25";
    });

    var childB = outer.layers.addSolid([0.1, 0.5, 0.9], "child B", 200, 200, 1, 10);
    step("parenting chain (child B)", function () { childB.parent = childA; });
    step("separated position", function () {
        var group = transform(childB);
        group.property("ADBE Position").dimensionsSeparated = true;
        var px = group.property("ADBE Position_0");
        var py = group.property("ADBE Position_1");
        px.setValueAtTime(0, 80);
        px.setValueAtTime(3, 520);
        py.setValueAtTime(0, 60);
        py.setValueAtTime(2, 300);
    });
    step("disabled expression", function () {
        var rot = transform(childB).property("ADBE Rotate Z");
        rot.expression = "time * 45";
        rot.expressionEnabled = false;
    });

    // --- the effects and masks host ---------------------------------------
    var fxHost = outer.layers.addSolid([0.6, 0.6, 0.6], "fx host", 640, 360, 1, 10);
    step("mask 1 (add)", function () {
        var m = fxHost.property("ADBE Mask Parade").addProperty("ADBE Mask Atom");
        m.name = "mask add";
        m.property("ADBE Mask Shape").setValue(shape([[40, 40], [280, 40], [280, 200], [40, 200]]));
        m.property("ADBE Mask Opacity").setValue(80);
        m.property("ADBE Mask Offset").setValue(4);
    });
    step("mask 2 (subtract, inverted, feathered, animated path)", function () {
        var m = fxHost.property("ADBE Mask Parade").addProperty("ADBE Mask Atom");
        m.name = "mask subtract";
        m.maskMode = MaskMode.SUBTRACT;
        m.inverted = true;
        m.property("ADBE Mask Feather").setValue([12, 12]);
        var path = m.property("ADBE Mask Shape");
        path.setValueAtTime(0, shape([[320, 80], [520, 80], [520, 260], [320, 260]]));
        path.setValueAtTime(2, shape([[340, 100], [560, 60], [540, 300], [300, 280]]));
    });
    step("layer markers", function () {
        var markers = fxHost.property("ADBE Marker");
        var mv = new MarkerValue("fx marker");
        mv.duration = 0.5;
        markers.setValueAtTime(1, mv);
        markers.setValueAtTime(3, new MarkerValue("second marker"));
    });

    var parade = fxHost.property("ADBE Effect Parade");
    function addFx(matchName) { return parade.addProperty(matchName); }

    step("Gaussian Blur (keyframed)", function () {
        var fx = addFx("ADBE Gaussian Blur 2");
        var blur = fx.property("ADBE Gaussian Blur 2-0001");
        blur.setValueAtTime(0, 0);
        blur.setValueAtTime(2, 40);
    });
    step("Tint (disabled instance)", function () {
        var fx = addFx("ADBE Tint");
        fx.property("ADBE Tint-0001").setValue([0, 0, 0.2, 1]);
        fx.property("ADBE Tint-0002").setValue([1, 0.9, 0.5, 1]);
        fx.property("ADBE Tint-0003").setValue(60);
        fx.enabled = false;
    });
    step("Fill", function () {
        var fx = addFx("ADBE Fill");
        fx.property("ADBE Fill-0002").setValue([0, 1, 0.25, 1]);
        fx.property("ADBE Fill-0005").setValue(0.8);
    });
    step("Transform", function () {
        var fx = addFx("ADBE Geometry2");
        fx.property("ADBE Geometry2-0002").setValue([200, 150]);
        fx.property("ADBE Geometry2-0007").setValue(15);
        fx.property("ADBE Geometry2-0008").setValue(90);
    });
    step("Fractal Noise (choice params)", function () {
        var fx = addFx("ADBE Fractal Noise");
        fx.property("ADBE Fractal Noise-0001").setValue(2);
        fx.property("ADBE Fractal Noise-0002").setValue(1);
        fx.property("ADBE Fractal Noise-0004").setValue(140);
        fx.property("ADBE Fractal Noise-0015").setValue(3);
        fx.property("ADBE Fractal Noise-0023").setValue(90);
    });
    step("Levels (histogram is a second unreadable)", function () {
        var fx = addFx("ADBE Easy Levels2");
        fx.property("ADBE Easy Levels2-0003").setValue(0.05);
        fx.property("ADBE Easy Levels2-0005").setValue(1.2);
    });
    step("Hue/Saturation", function () {
        var fx = addFx("ADBE HUE SATURATION");
        fx.property("ADBE HUE SATURATION-0004").setValue(30);
        fx.property("ADBE HUE SATURATION-0005").setValue(15);
    });
    // Curves is the unreadable: its point list is CUSTOM_VALUE data that AE's
    // own scripting DOM will not hand over. Nothing to set.
    step("Curves (the unreadable)", function () { addFx("ADBE CurvesCustom"); });
    step("Drop Shadow", function () {
        var fx = addFx("ADBE Drop Shadow");
        fx.property("ADBE Drop Shadow-0002").setValue(180);
        fx.property("ADBE Drop Shadow-0003").setValue(200);
        fx.property("ADBE Drop Shadow-0004").setValue(12);
        fx.property("ADBE Drop Shadow-0005").setValue(8);
    });
    step("Vegas (mask source)", function () {
        var fx = addFx("APC Vegas");
        fx.property("APC Vegas-0052").setValue(2);   // Stroke: Mask/Path
        fx.property("APC Vegas-0050").setValue(1);   // Path: mask 1
        fx.property("APC Vegas-0020").setValue(6);
        fx.property("APC Vegas-0028").setValue(8);
    });
    step("Scribble (mask reference)", function () {
        var fx = addFx("ADBE Scribble Fill");
        fx.property("ADBE Scribble Fill-0002").setValue(1);
        fx.property("ADBE Scribble Fill-0010").setValue(45);
        fx.property("ADBE Scribble Fill-0008").setValue(4);
    });
    // One match name Lumit does not ship, for the placeholder path.
    step("Invert (the unmapped match name)", function () { addFx("ADBE Invert"); });

    // --- retiming ---------------------------------------------------------
    var retimed = outer.layers.add(inner);
    retimed.name = "retimed precomp";
    step("collapse and frame blending (Frame Mix)", function () {
        retimed.collapseTransformation = true;
        retimed.frameBlendingType = FrameBlendingType.FRAME_MIX;
    });
    step("time remap with a hold key", function () {
        retimed.timeRemapEnabled = true;
        var tr = retimed.property("ADBE Time Remapping");
        tr.setValueAtTime(2, 1.0);
        var key = tr.nearestKeyIndex(2);
        tr.setInterpolationTypeAtKey(key, KeyframeInterpolationType.HOLD,
            KeyframeInterpolationType.HOLD);
    });

    var stretched = outer.layers.add(inner);
    stretched.name = "stretched precomp";
    step("stretch 50% and frame blending (Pixel Motion)", function () {
        stretched.stretch = 50;
        stretched.frameBlendingType = FrameBlendingType.PIXEL_MOTION;
    });

    var reversed = outer.layers.addSolid([0.4, 0.2, 0.6], "reversed", 200, 200, 1, 10);
    step("stretch -100%", function () { reversed.stretch = -100; });

    // --- the rest of the stack --------------------------------------------
    var adjustment = outer.layers.addSolid([1, 1, 1], "adjustment", 640, 360, 1, 10);
    step("adjustment layer", function () {
        adjustment.adjustmentLayer = true;
        adjustment.property("ADBE Effect Parade").addProperty("ADBE Gaussian Blur 2")
            .property("ADBE Gaussian Blur 2-0001").setValue(6);
    });

    var guide = outer.layers.addSolid([0, 0.5, 1], "guide", 640, 360, 1, 10);
    step("guide layer", function () { guide.guideLayer = true; });

    var threeD = outer.layers.addSolid([0.9, 0.9, 0.9], "3d card", 300, 200, 1, 10);
    step("3D layer", function () {
        threeD.threeDLayer = true;
        transform(threeD).property("ADBE Position").setValue([320, 180, -150]);
        transform(threeD).property("ADBE Orientation").setValue([0, 30, 0]);
        threeD.property("ADBE Material Options Group")
            .property("ADBE Casts Shadows").setValue(1);
    });

    var text = outer.layers.addText("Lumit fixture");
    step("text styling", function () {
        var source = text.property("ADBE Text Properties").property("ADBE Text Document");
        var doc = source.value;
        doc.fontSize = 48;
        doc.applyFill = true;
        doc.fillColor = [1, 0.55, 0.1];
        doc.applyStroke = true;
        doc.strokeColor = [0, 0, 0];
        doc.strokeWidth = 3;
        doc.tracking = 20;
        doc.justification = ParagraphJustification.CENTER_JUSTIFY;
        source.setValue(doc);
        // The font is deliberately left at AE's default: naming one would make
        // the fixture depend on what is installed.
    });

    var shapeLayer = outer.layers.addShape();
    shapeLayer.name = "shape";
    step("shape contents (rectangle, ellipse, gradient fill, Trim Paths)", function () {
        var root = shapeLayer.property("ADBE Root Vectors Group");
        var group = root.addProperty("ADBE Vector Group");
        var contents = group.property("ADBE Vectors Group");
        var rect = contents.addProperty("ADBE Vector Shape - Rect");
        rect.property("ADBE Vector Rect Size").setValue([200, 120]);
        rect.property("ADBE Vector Rect Position").setValue([-60, 0]);
        var ellipse = contents.addProperty("ADBE Vector Shape - Ellipse");
        ellipse.property("ADBE Vector Ellipse Size").setValue([120, 120]);
        ellipse.property("ADBE Vector Ellipse Position").setValue([80, 0]);
        // The gradient's colour stops are CUSTOM_VALUE data - a third unreadable.
        var fill = contents.addProperty("ADBE Vector Graphic - G-Fill");
        fill.property("ADBE Vector Grad Start Pt").setValue([-100, 0]);
        fill.property("ADBE Vector Grad End Pt").setValue([100, 0]);
        var trim = contents.addProperty("ADBE Vector Filter - Trim");
        trim.property("ADBE Vector Trim End").setValue(70);
    });
    step("shape Repeater", function () {
        var repeater = shapeLayer.property("ADBE Root Vectors Group")
            .addProperty("ADBE Vector Filter - Repeater");
        repeater.property("ADBE Vector Repeater Copies").setValue(3);
    });

    var alphaMatteSource = outer.layers.addSolid([1, 1, 1], "alpha matte source", 400, 300, 1, 10);
    step("modern alpha matte (inverted), naming a non-adjacent layer", function () {
        if (alphaMatted.setTrackMatte) {
            alphaMatted.setTrackMatte(alphaMatteSource, TrackMatteType.ALPHA_INVERTED);
        } else {
            alphaMatted.trackMatteType = TrackMatteType.ALPHA_INVERTED;
        }
    });

    var light = outer.layers.addLight("key light", [320, 180]);
    step("light", function () {
        light.lightType = LightType.SPOT;
        var options = light.property("ADBE Light Options Group");
        options.property("ADBE Light Intensity").setValue(75);
        options.property("ADBE Light Color").setValue([1, 0.9, 0.8]);
        options.property("ADBE Light Cone Angle").setValue(60);
        options.property("ADBE Casts Shadows").setValue(1);
    });

    var camera = outer.layers.addCamera("camera", [320, 180]);
    step("two-node camera", function () {
        camera.property("ADBE Camera Options Group").property("ADBE Camera Zoom").setValue(800);
        camera.property("ADBE Camera Options Group").property("ADBE Camera Depth of Field").setValue(1);
        camera.property("ADBE Camera Options Group").property("ADBE Camera Aperture").setValue(40);
    });

    step("comp markers", function () {
        var mv = new MarkerValue("comp marker");
        mv.duration = 1.25;
        outer.markerProperty.setValueAtTime(2, mv);
    });

    // Switches last, and the lock very last: a locked layer refuses edits.
    step("switches", function () {
        blendOverlay.shy = true;
        outer.hideShyLayers = true;
        childB.solo = true;
        blendDissolve.quality = LayerQuality.DRAFT;
        blendScreen.motionBlur = true;
        blendMultiply.label = 9;
        lumaMatteSource.enabled = false;
    });
    step("lock", function () { bg.locked = true; });

    app.endUndoGroup();

    // --- save, then walk --------------------------------------------------
    if (!fixtures.exists) fixtures.create();
    app.project.save(new File(fixtures.fsName + "/fixture.aep"));

    // Reuse the walker rather than repeating it: the flag has to live on the
    // global object, because $.evalFile evaluates in global scope.
    $.global.LUMIT_BRIDGE_EMBED = true;
    $.evalFile(new File(toolFolder.fsName + "/lumit-bridge.jsx"));
    // Clear it again, or running the walker on its own later in this same AE
    // session would find the flag still set and skip its dialog.
    $.global.LUMIT_BRIDGE_EMBED = false;
    var summary = $.global.LumitBridge.exportBundle(
        new Folder(fixtures.fsName + "/fixture.lum-bundle"));

    alert("Lumit fixture built.\n\n" +
        "Project: " + fixtures.fsName + "/fixture.aep\n" +
        "Bundle:  " + summary.folder.fsName + "\n\n" +
        "Items: " + summary.items +
        "   Comps: " + summary.comps +
        "   Unreadable properties: " + summary.unreadables +
        (problems.length > 0
            ? "\n\nSteps that did not apply (" + problems.length + "):\n" + problems.join("\n")
            : "\n\nEvery checklist row applied."));
})();
