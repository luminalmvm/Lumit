// Lumit Bridge fixture builder: Essential Graphics.
//
// In plain terms: this script builds one small After Effects project with a
// composition that has Essential Graphics controllers of each kind, and a
// second composition that places it twice, each layer setting some of its
// Essential Properties, one of them keyframed. It saves the project and runs
// the walker over it. The two files are what
// crates/lumit-import/tests/essential_graphics.rs reads, one by each way in.
//
// Every value is written and nothing is sampled from the clock. The footage
// is a placeholder, so no file path is recorded. The text names no font, so
// After Effects' default comes through.
//
// How to run (After Effects 2024+):
//   Save and close whatever you are working on first: this builds a NEW project.
//   Edit > Preferences > Scripting & Expressions > "Allow Scripts to Write Files
//   and Access Network" must be on.
//   File > Scripts > Run Script File...  ->  pick this file
// It writes crates/lumit-import/tests/fixtures/essential-graphics.aep and
// essential-graphics.lum-bundle/ beside it.
//
// Groups and comments are not here: scripting can add a property to
// Essential Graphics and cannot add either of those.

(function makeEssentialFixture() {
    var toolFolder = new File($.fileName).parent;
    var fixtures = new Folder(toolFolder.parent.parent.fsName + "/crates/lumit-import/tests/fixtures");
    // Set by a caller that runs this unattended, so no dialog waits for a hand.
    var quiet = $.global.LUMIT_FIXTURE_QUIET === true;

    var problems = [];
    function step(what, fn) {
        try { fn(); } catch (err) { problems.push(what + " - " + String(err)); }
    }

    if (!quiet && !confirm("Build the Essential Graphics fixture project?\n\n" +
        "This creates a NEW After Effects project - save anything open first.")) {
        return;
    }
    if (!app.newProject()) return;

    app.beginUndoGroup("Lumit Essential Graphics fixture");

    // --- the template: bottom of the stack first -----------------------------
    var lower = app.project.items.addComp("Lower third", 640, 360, 1, 5, 25);

    var plate = lower.layers.addSolid([0.1, 0.2, 0.4], "Plate", 640, 120, 1, 5);
    function fx() { return lower.layer("Plate").property("ADBE Effect Parade"); }
    step("Gaussian Blur", function () {
        fx().addProperty("ADBE Gaussian Blur 2").property("ADBE Gaussian Blur 2-0001").setValue(4);
    });
    step("Checkbox Control", function () {
        var e = fx().addProperty("ADBE Checkbox Control");
        e.name = "Show plate";
        e.property(1).setValue(1);
    });
    step("Dropdown Menu Control", function () {
        var e = fx().addProperty("ADBE Dropdown Control");
        // Setting the options replaces the effect, so it is named afterwards.
        var menu = e.property(1).setPropertyParameters(["Left", "Centre", "Right"]);
        menu.parentProperty.name = "Side";
        fx().property("Side").property(1).setValue(2);
    });
    step("Slider Control", function () {
        var e = fx().addProperty("ADBE Slider Control");
        e.name = "Amount";
        e.property(1).setValue(25);
    });
    step("Point Control", function () {
        var e = fx().addProperty("ADBE Point Control");
        e.name = "Spot";
        e.property(1).setValue([320, 60]);
    });
    step("Color Control", function () {
        var e = fx().addProperty("ADBE Color Control");
        e.name = "Tint";
        e.property(1).setValue([1, 0.5, 0, 1]);
    });
    step("Angle Control", function () {
        var e = fx().addProperty("ADBE Angle Control");
        e.name = "Turn";
        e.property(1).setValue(30);
    });
    step("Fill", function () {
        fx().addProperty("ADBE Fill").property("ADBE Fill-0002").setValue([0, 0.5, 1, 1]);
    });

    var bar = lower.layers.addShape();
    bar.name = "Bar";
    step("shape contents", function () {
        var group = bar.property("ADBE Root Vectors Group").addProperty("ADBE Vector Group");
        group.name = "Bar shape";
        var contents = group.property("ADBE Vectors Group");
        contents.addProperty("ADBE Vector Shape - Rect").property("ADBE Vector Rect Size").setValue([300, 12]);
        contents.addProperty("ADBE Vector Graphic - Fill").property("ADBE Vector Fill Color").setValue([1, 0.8, 0.1, 1]);
    });

    var title = lower.layers.addText("Name here");
    title.name = "Title";
    step("text size", function () {
        var source = title.property("ADBE Text Properties").property("ADBE Text Document");
        var doc = source.value;
        doc.fontSize = 40;
        doc.fillColor = [1, 1, 1];
        source.setValue(doc);
    });

    var logoItem = app.project.importPlaceholder("Logo", 200, 100, 25, 5);
    var logo = lower.layers.add(logoItem);
    logo.name = "Logo";

    // --- its controllers: each new one lands at the top of the list ----------
    function add(what, prop, name) {
        step(what, function () { prop().addToMotionGraphicsTemplateAs(lower, name); });
    }
    function control(name) { return function () { return fx().property(name).property(1); }; }
    function transform(layer, name) {
        return function () { return lower.layer(layer).property("ADBE Transform Group").property(name); };
    }
    function vector(index, name) {
        return function () {
            return lower.layer("Bar").property("ADBE Root Vectors Group").property(1)
                .property("ADBE Vectors Group").property(index).property(name);
        };
    }
    add("media", function () { return lower.layer("Logo"); }, "Logo");
    // A rectangle's size is one row in After Effects and two in Lumit.
    add("rect size", vector(1, "ADBE Vector Rect Size"), "Bar size");
    add("fill effect colour", function () { return fx().property("ADBE Fill").property("ADBE Fill-0002"); }, "Plate colour");
    add("slider control", control("Amount"), "Amount");
    add("angle control", control("Turn"), "Turn");
    add("colour control", control("Tint"), "Tint");
    add("point control", control("Spot"), "Spot");
    add("scale", transform("Title", "ADBE Scale"), "Title scale");
    add("dropdown", control("Side"), "Side");
    add("rotation", transform("Bar", "ADBE Rotate Z"), "Bar angle");
    add("position", transform("Bar", "ADBE Position"), "Bar position");
    add("checkbox control", control("Show plate"), "Show plate");
    add("effect slider", function () { return fx().property(1).property("ADBE Gaussian Blur 2-0001"); }, "Blur");
    add("opacity", transform("Bar", "ADBE Opacity"), "Bar opacity");
    add("shape fill", vector(2, "ADBE Vector Fill Color"), "Bar colour");
    add("source text", function () {
        return lower.layer("Title").property("ADBE Text Properties").property("ADBE Text Document");
    }, "Title");
    step("template name", function () { lower.motionGraphicsTemplateName = "Lower third"; });

    // --- two layers placing it, each setting some of them --------------------
    var main = app.project.items.addComp("Main", 640, 360, 1, 5, 25);
    var b = main.layers.add(lower);
    b.name = "Copy B";
    var a = main.layers.add(lower);
    a.name = "Copy A";
    // Copy A starts a second in, so its keys are a second apart on the two clocks.
    step("copy A start", function () { a.startTime = 1; });

    function own(layer, name) { return layer.property("ADBE Layer Overrides").property(name); }
    step("A title", function () {
        var p = own(a, "Title");
        var doc = p.value;
        doc.text = "Ada Lovelace";
        p.setValue(doc);
    });
    step("A bar colour", function () { own(a, "Bar colour").setValue([0.2, 0.9, 0.3, 1]); });
    step("A bar opacity keys", function () {
        var p = own(a, "Bar opacity");
        p.setValueAtTime(1, 0);
        p.setValueAtTime(2, 100);
    });
    step("A side", function () { own(a, "Side").setValue(3); });
    step("A expression", function () {
        a.property("ADBE Transform Group").property("ADBE Rotate Z").expression =
            'thisLayer.essentialProperty("Amount") * 2';
    });

    step("B title", function () {
        var p = own(b, "Title");
        var doc = p.value;
        doc.text = "Grace Hopper";
        p.setValue(doc);
    });
    step("B show plate", function () { own(b, "Show plate").setValue(0); });
    step("B bar position", function () { own(b, "Bar position").setValue([100, 300]); });
    step("B bar angle", function () { own(b, "Bar angle").setValue(45); });
    step("B blur", function () { own(b, "Blur").setValue(12); });
    step("B amount", function () { own(b, "Amount").setValue(60); });
    step("B tint", function () { own(b, "Tint").setValue([0, 1, 0, 1]); });
    step("B spot", function () { own(b, "Spot").setValue([10, 20]); });
    step("B plate colour", function () { own(b, "Plate colour").setValue([1, 0, 0, 1]); });
    step("B title scale", function () { own(b, "Title scale").setValue([50, 50]); });
    step("B logo", function () {
        var other = app.project.importPlaceholder("Other logo", 300, 150, 25, 5);
        own(b, "Logo").setAlternateSource(other);
    });

    app.endUndoGroup();

    // --- save, then walk ------------------------------------------------------
    if (!fixtures.exists) fixtures.create();
    app.project.save(new File(fixtures.fsName + "/essential-graphics.aep"));

    $.global.LUMIT_BRIDGE_EMBED = true;
    $.evalFile(new File(toolFolder.fsName + "/lumit-bridge.jsx"));
    $.global.LUMIT_BRIDGE_EMBED = false;
    var summary = $.global.LumitBridge.exportBundle(
        new Folder(fixtures.fsName + "/essential-graphics.lum-bundle"));

    var said = "Essential Graphics fixture built.\n\n" +
        "Project: " + fixtures.fsName + "/essential-graphics.aep\n" +
        "Bundle:  " + summary.folder.fsName + "\n\n" +
        "Items: " + summary.items + "   Comps: " + summary.comps +
        (problems.length > 0
            ? "\n\nSteps that did not apply (" + problems.length + "):\n" + problems.join("\n")
            : "\n\nEvery step applied.");
    $.global.LUMIT_FIXTURE_SAID = said;
    if (!quiet) alert(said);
})();
