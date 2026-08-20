// Lumit AE-import audit (docs/11-AE-IMPORT.md's live-AE verification rule).
//
// In plain terms: the import spec's effect table was written from documentation
// and memory, and its own rule says no match name is trusted until a real
// After Effects has confirmed it. This script is that confirmation, in one
// sitting: run it inside AE and it writes a JSON report beside itself with,
// for every effect AE actually ships, the match name, display name, category,
// and every property's match name, name, type and default - plus a pass over
// Lumit's claimed list saying found / missing / renamed-suspect.
//
// How to run (After Effects 2024+, any project open or none):
//   File > Scripts > Run Script File...  ->  pick this file
//   (If it refuses file writes: Edit > Preferences > Scripting & Expressions >
//    "Allow Scripts to Write Files and Access Network" must be on.)
// Output: ae-audit-report.json next to this script. Takes ~1-2 minutes.
//
// The script builds a throwaway project item (a comp with one solid), applies
// effects to read their property trees, and deletes everything it made - your
// open project is not touched beyond an undo group.

(function auditLumitImport() {
    var scriptFile = new File($.fileName);
    var outFile = new File(scriptFile.parent.fsName + "/ae-audit-report.json");
    var claimedFile = new File(scriptFile.parent.fsName + "/claimed-matchnames.txt");

    // --- read the claimed list -------------------------------------------
    var claimed = [];
    if (claimedFile.exists) {
        claimedFile.open("r");
        while (!claimedFile.eof) {
            var line = claimedFile.readln();
            if (line !== "") claimed.push(line);
        }
        claimedFile.close();
    }

    // --- JSON encoding (ExtendScript has no JSON object) ------------------
    function esc(s) {
        return String(s)
            .replace(/\\/g, "\\\\").replace(/"/g, '\\"')
            .replace(/\n/g, "\\n").replace(/\r/g, "\\r").replace(/\t/g, "\\t");
    }
    function q(s) { return '"' + esc(s) + '"'; }

    app.beginUndoGroup("Lumit import audit");
    var comp = app.project.items.addComp("lumit_audit_tmp", 320, 240, 1, 1, 30);
    var solid = comp.layers.addSolid([0.5, 0.5, 0.5], "s", 320, 240, 1, 1);

    // --- every installed effect ------------------------------------------
    var out = [];
    out.push("{");
    out.push(q("ae_version") + ": " + q(app.version) + ",");
    out.push(q("effects") + ": [");
    var effectRows = [];
    var byMatch = {};
    // app.effects is a plain 0-based JS array, not a 1-based AE Collection.
    for (var i = 0; i < app.effects.length; i++) {
        var e = app.effects[i];
        if (!e || !e.matchName) continue;
        byMatch[e.matchName] = e;
        effectRows.push(
            "{" + q("match_name") + ": " + q(e.matchName) +
            ", " + q("name") + ": " + q(e.displayName) +
            ", " + q("category") + ": " + q(e.category) + "}"
        );
    }
    out.push(effectRows.join(",\n"));
    out.push("],");

    // --- property trees for the claimed set (and only it - applying all
    //     ~400 effects is slow and some legacy ones dialog on apply) --------
    function propType(p) {
        switch (p.propertyValueType) {
            case PropertyValueType.OneD: return "float";
            case PropertyValueType.TwoD: case PropertyValueType.TwoD_SPATIAL: return "point";
            case PropertyValueType.ThreeD: case PropertyValueType.ThreeD_SPATIAL: return "point3";
            case PropertyValueType.COLOR: return "colour";
            case PropertyValueType.CUSTOM_VALUE: return "custom_blob";
            case PropertyValueType.LAYER_INDEX: return "layer";
            case PropertyValueType.MASK_INDEX: return "mask";
            case PropertyValueType.SHAPE: return "shape";
            case PropertyValueType.TEXT_DOCUMENT: return "text";
            case PropertyValueType.NO_VALUE: return "group";
            default: return "other";
        }
    }
    function valueOf(p) {
        try {
            var v = p.value;
            if (v instanceof Array) return "[" + v.join(",") + "]";
            return String(v);
        } catch (err) { return null; }
    }
    out.push(q("claimed") + ": [");
    var claimRows = [];
    for (var c = 0; c < claimed.length; c++) {
        var mn = claimed[c];
        var row = "{" + q("match_name") + ": " + q(mn) + ", ";
        if (!byMatch[mn]) {
            // Not installed under that exact name - hunt for a suspect by
            // display-name similarity so the report names the likely rename.
            var suspect = "";
            var tail = mn.replace(/^ADBE /, "").replace(/\d+$/, "").toLowerCase();
            for (var s in byMatch) {
                if (s.toLowerCase().indexOf(tail.substring(0, Math.max(4, tail.length - 2))) !== -1 && s !== mn) {
                    suspect = s; break;
                }
            }
            row += q("status") + ": " + q("missing") + ", " + q("suspect") + ": " + q(suspect) + "}";
            claimRows.push(row);
            continue;
        }
        row += q("status") + ": " + q("found") + ", " + q("name") + ": " + q(byMatch[mn].displayName) + ", " + q("properties") + ": [";
        var props = [];
        try {
            var fx = solid.property("ADBE Effect Parade").addProperty(mn);
            for (var p = 1; p <= fx.numProperties; p++) {
                var prop = fx.property(p);
                var pv = valueOf(prop);
                props.push(
                    "{" + q("match_name") + ": " + q(prop.matchName) +
                    ", " + q("name") + ": " + q(prop.name) +
                    ", " + q("type") + ": " + q(propType(prop)) +
                    (pv !== null ? ", " + q("default") + ": " + q(pv) : "") + "}"
                );
            }
            fx.remove();
        } catch (err) {
            props.push("{" + q("error") + ": " + q(String(err)) + "}");
        }
        row += props.join(",\n") + "]}";
        claimRows.push(row);
    }
    out.push(claimRows.join(",\n"));
    out.push("]}");

    comp.remove();
    app.endUndoGroup();

    outFile.open("w");
    outFile.encoding = "UTF-8";
    outFile.write(out.join("\n"));
    outFile.close();

    alert("Lumit audit written:\n" + outFile.fsName +
          "\n\nEffects listed: " + app.effects.length +
          "\nClaimed names checked: " + claimed.length);
})();
