// Lumit Bridge - the After Effects export walk.
//
// In plain terms: this script runs inside After Effects and writes down
// everything the project contains - items, comps, layers, keyframes, masks,
// effects, expressions - into a folder of JSON files. It is a courier, not a
// translator: it records what the scripting DOM said, in AE's own words, with
// AE's own ids and AE's own float seconds. Every conversion happens later, in
// Rust, where the regression suite can cover it.
//
// Spec: docs/11-AE-IMPORT.md section 2.2 (the walk). Capture schema:
// docs/impl/ae-import.md section 2. Traps: section 3 of the same note.
//
// How to run (After Effects 2024+, with the project you want exported open):
//   File > Scripts > Run Script File...  ->  pick this file
//   (If it refuses file writes: Edit > Preferences > Scripting & Expressions >
//    "Allow Scripts to Write Files and Access Network" must be on.)
// It asks where to put the bundle, defaulting beside the project, and writes
// <ProjectName>.lum-bundle/ containing manifest.json, capture.json, report.json.
//
// make-fixture.jsx reuses the walk: it sets $.global.LUMIT_BRIDGE_EMBED before
// $.evalFile-ing this file, which suppresses the dialog, then calls
// $.global.LumitBridge.exportBundle(folder).

var LumitBridge = (function () {
    var BUNDLE_VERSION = "1.0.0";
    var BRIDGE_VERSION = "1.0.0";

    // The walk's running state. Reset by exportBundle.
    var report = { unreadables: [] };
    var here = { comp: "", layer: "" };

    // --- JSON encoding (ExtendScript has no JSON object) ------------------
    // Same escaper as tools/ae-audit/audit.jsx, plus a sweep for the remaining
    // control characters so a stray one in a comment cannot produce a file no
    // parser will read.
    function esc(s) {
        return String(s)
            .replace(/\\/g, "\\\\").replace(/"/g, '\\"')
            .replace(/\n/g, "\\n").replace(/\r/g, "\\r").replace(/\t/g, "\\t")
            .replace(/[\x00-\x1f]/g, function (c) {
                var h = c.charCodeAt(0).toString(16);
                return "\\u00" + (h.length < 2 ? "0" + h : h);
            });
    }
    function q(s) { return '"' + esc(s) + '"'; }
    function num(n) {
        if (typeof n !== "number" || isNaN(n) || !isFinite(n)) return "null";
        return String(n);
    }
    // Serialises plain data only - never an AE DOM object. Everything the walk
    // stores is built here as plain objects, arrays, numbers, strings, booleans.
    // undefined members are dropped, which is how the "capture it if the DOM
    // has it" fields stay out of the file when the DOM does not have them.
    function jsonOf(v, indent) {
        if (v === null || v === undefined) return "null";
        var t = typeof v;
        if (t === "number") return num(v);
        if (t === "boolean") return v ? "true" : "false";
        if (t === "string") return q(v);
        var pad = indent + "  ";
        var parts = [];
        var i, k;
        if (v instanceof Array) {
            if (v.length === 0) return "[]";
            for (i = 0; i < v.length; i++) parts.push(pad + jsonOf(v[i], pad));
            return "[\n" + parts.join(",\n") + "\n" + indent + "]";
        }
        for (k in v) {
            if (!v.hasOwnProperty(k)) continue;
            if (v[k] === undefined) continue;
            parts.push(pad + q(k) + ": " + jsonOf(v[k], pad));
        }
        if (parts.length === 0) return "{}";
        return "{\n" + parts.join(",\n") + "\n" + indent + "}";
    }
    function writeJson(file, obj) {
        file.encoding = "UTF-8";
        if (!file.open("w")) throw new Error("cannot write " + file.fsName);
        file.write(jsonOf(obj, ""));
        file.write("\n");
        file.close();
    }

    // --- small helpers ----------------------------------------------------
    // attempt() is for reads the DOM refuses by design: a spatial tangent on a
    // scalar, a box-text field on point text, a switch a camera layer lacks.
    // Those are not failures and do not belong in the report; they simply do
    // not appear in the capture. A *property value* that cannot be read is a
    // different thing and goes through addUnreadable.
    function attempt(fn) {
        try { return fn(); } catch (err) { return undefined; }
    }
    function addUnreadable(path, matchName, error) {
        report.unreadables.push({
            comp: here.comp, layer: here.layer,
            path: path, match_name: matchName, error: error
        });
    }
    // Enum values come back as opaque numbers; name them from a list of the
    // constants we know. An unknown name yields undefined and is skipped, so
    // this survives Adobe adding or removing entries in either direction.
    function enumName(v, obj, names) {
        if (v === undefined || v === null) return undefined;
        for (var i = 0; i < names.length; i++) {
            var c = obj[names[i]];
            if (c !== undefined && c === v) return names[i];
        }
        return String(v);
    }
    function numArray(a) {
        if (a === undefined || a === null) return undefined;
        var out = [];
        for (var i = 0; i < a.length; i++) out.push(Number(a[i]));
        return out;
    }
    function pointList(a) {
        var out = [];
        for (var i = 0; i < a.length; i++) out.push([Number(a[i][0]), Number(a[i][1])]);
        return out;
    }
    function today() {
        var d = new Date();
        var m = d.getMonth() + 1, day = d.getDate();
        return d.getFullYear() + "-" + (m < 10 ? "0" + m : m) + "-" + (day < 10 ? "0" + day : day);
    }

    var BLEND_NAMES = ["NORMAL", "DISSOLVE", "DANCING_DISSOLVE", "DARKEN", "MULTIPLY",
        "COLOR_BURN", "CLASSIC_COLOR_BURN", "LINEAR_BURN", "DARKER_COLOR", "ADD",
        "LIGHTEN", "SCREEN", "COLOR_DODGE", "CLASSIC_COLOR_DODGE", "LINEAR_DODGE",
        "LIGHTER_COLOR", "OVERLAY", "SOFT_LIGHT", "HARD_LIGHT", "LINEAR_LIGHT",
        "VIVID_LIGHT", "PIN_LIGHT", "HARD_MIX", "DIFFERENCE", "CLASSIC_DIFFERENCE",
        "EXCLUSION", "SUBTRACT", "DIVIDE", "HUE", "SATURATION", "COLOR", "LUMINOSITY",
        "STENCIL_ALPHA", "STENCIL_LUMA", "SILHOUETTE_ALPHA", "SILHOUETTE_LUMA",
        "ALPHA_ADD", "LUMINESCENT_PREMUL"];
    var MATTE_NAMES = ["NO_TRACK_MATTE", "ALPHA", "ALPHA_INVERTED", "LUMA", "LUMA_INVERTED"];
    var MASK_MODE_NAMES = ["NONE", "ADD", "SUBTRACT", "INTERSECT", "LIGHTEN", "DARKEN", "DIFFERENCE"];
    var INTERP_NAMES = ["LINEAR", "BEZIER", "HOLD"];
    var QUALITY_NAMES = ["WIREFRAME", "DRAFT", "BEST"];
    var FRAME_BLEND_NAMES = ["NO_FRAME_BLEND", "FRAME_MIX", "PIXEL_MOTION"];
    var AUTO_ORIENT_NAMES = ["NO_AUTO_ORIENT", "ALONG_PATH", "CAMERA_OR_POINT_OF_INTEREST",
        "CHARACTERS_TOWARD_CAMERA"];
    var LIGHT_TYPE_NAMES = ["PARALLEL", "SPOT", "POINT", "AMBIENT"];
    var ALPHA_NAMES = ["IGNORE", "STRAIGHT", "PREMULTIPLIED"];
    var FIELD_NAMES = ["OFF", "UPPER_FIELD_FIRST", "LOWER_FIELD_FIRST"];

    // Same mapping as audit.jsx, so the two kits describe a property the same way.
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
            case PropertyValueType.MARKER: return "marker";
            case PropertyValueType.NO_VALUE: return "group";
            default: return "other";
        }
    }

    // --- values -----------------------------------------------------------
    function shapeValue(s) {
        return {
            vertices: pointList(s.vertices),
            in_tangents: pointList(s.inTangents),
            out_tangents: pointList(s.outTangents),
            closed: s.closed === true
        };
    }

    // Every TextDocument attribute throws when it does not apply to this text
    // layer (box fields on point text, per-character-3D fields when off), so
    // each gets its own attempt. fontLocation is deliberately not captured: it
    // is a path on the exporting machine and carries nothing an import needs.
    var TEXT_ATTRS = ["text", "font", "fontFamily", "fontStyle", "fontSize",
        "applyFill", "applyStroke", "fillColor", "strokeColor", "strokeWidth",
        "strokeOverFill", "justification", "tracking", "leading", "autoLeading",
        "baselineShift", "tsume", "boxText", "boxTextPos", "boxTextSize",
        "allCaps", "smallCaps", "superscript", "subscript",
        "verticalScale", "horizontalScale", "fauxBold", "fauxItalic"];
    function textValue(td) {
        var out = {};
        for (var i = 0; i < TEXT_ATTRS.length; i++) {
            var key = TEXT_ATTRS[i];
            var v = attempt(function () { return td[key]; });
            if (v === undefined || v === null) continue;
            out[key] = (v instanceof Array) ? numArray(v) : v;
        }
        return out;
    }

    function coerce(v, valueType) {
        if (v === undefined || v === null) return null;
        if (valueType === "shape") return shapeValue(v);
        if (valueType === "text") return textValue(v);
        if (v instanceof Array) return numArray(v);
        var t = typeof v;
        if (t === "number" || t === "boolean" || t === "string") return v;
        return String(v);
    }

    // --- keyframes (docs/11 section 2.2 item 5, in full) -------------------------
    function easeList(arr) {
        if (arr === undefined || arr === null) return undefined;
        var out = [];
        for (var i = 0; i < arr.length; i++) {
            out.push({ speed: Number(arr[i].speed), influence: Number(arr[i].influence) });
        }
        return out;
    }
    function keyAt(prop, i, valueType) {
        var k = {};
        k.t = Number(prop.keyTime(i));
        k.v = coerce(prop.keyValue(i), valueType);
        k.in_interp = enumName(attempt(function () { return prop.keyInInterpolationType(i); }),
            KeyframeInterpolationType, INTERP_NAMES);
        k.out_interp = enumName(attempt(function () { return prop.keyOutInterpolationType(i); }),
            KeyframeInterpolationType, INTERP_NAMES);
        // Temporal ease is an array per dimension; a spatial property returns
        // exactly one entry. Captured as the DOM hands it over.
        k.in_ease = easeList(attempt(function () { return prop.keyInTemporalEase(i); }));
        k.out_ease = easeList(attempt(function () { return prop.keyOutTemporalEase(i); }));
        k.in_tangent = attempt(function () { return numArray(prop.keyInSpatialTangent(i)); });
        k.out_tangent = attempt(function () { return numArray(prop.keyOutSpatialTangent(i)); });
        k.roving = attempt(function () { return prop.keyRoving(i); });
        k.auto_bezier = attempt(function () { return prop.keyTemporalAutoBezier(i); });
        k.continuous = attempt(function () { return prop.keyTemporalContinuous(i); });
        k.spatial_auto_bezier = attempt(function () { return prop.keySpatialAutoBezier(i); });
        k.spatial_continuous = attempt(function () { return prop.keySpatialContinuous(i); });
        return k;
    }

    // --- the property tree ------------------------------------------------
    function maskFields(group) {
        var mode = attempt(function () { return group.maskMode; });
        if (mode === undefined) return undefined;
        return {
            mode: enumName(mode, MaskMode, MASK_MODE_NAMES),
            inverted: attempt(function () { return group.inverted; }),
            roto_bezier: attempt(function () { return group.rotoBezier; }),
            locked: attempt(function () { return group.locked; }),
            colour: attempt(function () { return numArray(group.color); })
        };
    }

    function walkGroup(group, path) {
        var out = [];
        var count = attempt(function () { return group.numProperties; });
        if (!count) return out;
        for (var i = 1; i <= count; i++) {
            var child = null;
            try {
                child = group.property(i);
            } catch (err) {
                addUnreadable(path + " > [" + i + "]", "", String(err));
                continue;
            }
            if (!child) continue;
            // Markers are captured as the layer's markers[] instead: their key
            // values are MarkerValue objects, not property values.
            if (attempt(function () { return child.matchName; }) === "ADBE Marker") continue;
            out.push(walkProperty(child, path));
        }
        return out;
    }

    function walkProperty(prop, parentPath) {
        var name = attempt(function () { return prop.name; });
        var matchName = attempt(function () { return prop.matchName; });
        var label = name || matchName || "?";
        var path = parentPath ? (parentPath + " > " + label) : label;

        var node = { match_name: matchName, name: name };

        var isGroup = attempt(function () { return prop.propertyType !== PropertyType.PROPERTY; });
        if (isGroup) {
            // Groups are where an enabled state means something: it is the
            // effect's on/off switch, and the mask's (docs/11 section 2.2 item 9).
            node.enabled = attempt(function () { return prop.enabled; });
            node.mask = maskFields(prop);
            node.group = walkGroup(prop, path);
            return node;
        }

        var valueType = attempt(function () { return propType(prop); });
        node.value_type = valueType;

        if (attempt(function () { return prop.canSetExpression; })) {
            var ex = attempt(function () { return prop.expression; });
            if (ex) {
                node.expression = ex;
                node.expression_enabled = attempt(function () { return prop.expressionEnabled; });
            }
        }

        // Separated dimensions: the leader's own keyframes are not the
        // animation - the followers carry it.
        if (attempt(function () { return prop.isSeparationLeader && prop.dimensionsSeparated; })) {
            var followers = [];
            // getSeparationFollower's dimension index is 0-based - one of the
            // few things in this DOM that is.
            for (var d = 0; d < 3; d++) {
                var f = attempt(function () { return prop.getSeparationFollower(d); });
                if (!f) break;
                followers.push(walkProperty(f, path));
            }
            node.separated = followers;
        }

        // One try/catch per property read. A failure becomes an unreadable
        // entry on the node and a row in report.json, and the walk continues.
        try {
            if (valueType === "custom_blob") {
                // AE's own scripting DOM cannot read CUSTOM_VALUE data (Curves'
                // point list, Levels' histogram, Hue/Saturation's channel
                // ranges). Say so rather than guess.
                throw new Error("CUSTOM_VALUE data is not readable from the scripting DOM");
            }
            var keys = prop.numKeys;
            if (keys > 0) {
                var frames = [];
                for (var k = 1; k <= keys; k++) frames.push(keyAt(prop, k, valueType));
                node.keyframes = frames;
            } else {
                node.value = coerce(prop.value, valueType);
            }
        } catch (err) {
            node.unreadable = String(err);
            addUnreadable(path, matchName, String(err));
        }
        return node;
    }

    // --- markers ----------------------------------------------------------
    function markersOf(markerProp) {
        var out = [];
        if (!markerProp) return out;
        var count = attempt(function () { return markerProp.numKeys; });
        if (!count) return out;
        for (var i = 1; i <= count; i++) {
            var mv = attempt(function () { return markerProp.keyValue(i); });
            if (!mv) continue;
            out.push({
                t: attempt(function () { return Number(markerProp.keyTime(i)); }),
                comment: attempt(function () { return mv.comment; }),
                duration: attempt(function () { return Number(mv.duration); }),
                chapter: attempt(function () { return mv.chapter; }),
                label: attempt(function () { return mv.label; })
            });
        }
        return out;
    }

    // --- items ------------------------------------------------------------
    function itemOf(item) {
        var node = {
            id: attempt(function () { return item.id; }),
            name: attempt(function () { return item.name; }),
            parent_id: attempt(function () { return item.parentFolder.id; })
        };
        if (item instanceof FolderItem) {
            node.kind = "folder";
            return node;
        }
        if (item instanceof CompItem) {
            node.kind = "comp";
            return node;
        }
        var source = attempt(function () { return item.mainSource; });
        if (source && source instanceof SolidSource) {
            node.kind = "solid";
            node.colour = attempt(function () { return numArray(source.color); });
            node.width = attempt(function () { return item.width; });
            node.height = attempt(function () { return item.height; });
            return node;
        }
        node.kind = "footage";
        node.path = attempt(function () { return source.file ? source.file.fsName : ""; });
        node.width = attempt(function () { return item.width; });
        node.height = attempt(function () { return item.height; });
        node.fps = attempt(function () { return item.frameRate; });
        node.duration = attempt(function () { return item.duration; });
        node.fps_override = attempt(function () { return source.conformFrameRate; });
        node.native_fps = attempt(function () { return source.nativeFrameRate; });
        node.alpha = enumName(attempt(function () { return source.alphaMode; }), AlphaMode, ALPHA_NAMES);
        node.premul_colour = attempt(function () { return numArray(source.premulColor); });
        node.invert_alpha = attempt(function () { return source.invertAlpha; });
        node.loop = attempt(function () { return source.loop; });
        node.fields = enumName(attempt(function () { return source.fieldSeparationType; }),
            FieldSeparationType, FIELD_NAMES);
        node.remove_pulldown = attempt(function () { return String(source.removePulldown); });
        node.is_still = attempt(function () { return source.isStill; });
        node.is_placeholder = attempt(function () { return source instanceof PlaceholderSource; });
        node.is_missing = attempt(function () { return item.footageMissing; });
        return node;
    }

    // --- layers -----------------------------------------------------------
    function layerKind(layer) {
        if (layer instanceof CameraLayer) return "camera";
        if (layer instanceof LightLayer) return "light";
        if (layer instanceof TextLayer) return "text";
        if (layer instanceof ShapeLayer) return "shape";
        if (attempt(function () { return layer.nullLayer; })) return "null";
        if (attempt(function () { return layer.adjustmentLayer; })) return "adjustment";
        var source = attempt(function () { return layer.source; });
        if (source) {
            if (source instanceof CompItem) return "precomp";
            var main = attempt(function () { return source.mainSource; });
            if (main && main instanceof SolidSource) return "solid";
            if (attempt(function () { return layer.hasVideo; }) === false) return "audio";
        }
        return "footage";
    }

    function layerOf(layer) {
        here.layer = attempt(function () { return layer.name; }) || "";
        var node = {
            index: attempt(function () { return layer.index; }),
            name: here.layer,
            kind: layerKind(layer),
            source_id: attempt(function () { return layer.source.id; }),
            in_point: attempt(function () { return layer.inPoint; }),
            out_point: attempt(function () { return layer.outPoint; }),
            start_time: attempt(function () { return layer.startTime; }),
            stretch: attempt(function () { return layer.stretch; }),
            parent_index: attempt(function () { return layer.parent ? layer.parent.index : null; }),
            label: attempt(function () { return layer.label; }),
            blend: enumName(attempt(function () { return layer.blendingMode; }), BlendingMode, BLEND_NAMES),
            preserve_transparency: attempt(function () { return layer.preserveTransparency; }),
            auto_orient: enumName(attempt(function () { return layer.autoOrient; }),
                AutoOrientType, AUTO_ORIENT_NAMES),
            light_type: enumName(attempt(function () { return layer.lightType; }), LightType, LIGHT_TYPE_NAMES),
            time_remap_enabled: attempt(function () { return layer.timeRemapEnabled; })
        };
        // Both matte generations: the 23.0+ selectable form names its matte
        // layer, the legacy form is the layer above and says only its type.
        // is_track_matte marks a layer being used as somebody's matte.
        node.matte = {
            type: enumName(attempt(function () { return layer.trackMatteType; }), TrackMatteType, MATTE_NAMES),
            layer_index: attempt(function () { return layer.trackMatteLayer.index; }),
            is_track_matte: attempt(function () { return layer.isTrackMatte; })
        };
        node.switches = {
            enabled: attempt(function () { return layer.enabled; }),
            audio: attempt(function () { return layer.audioEnabled; }),
            solo: attempt(function () { return layer.solo; }),
            lock: attempt(function () { return layer.locked; }),
            shy: attempt(function () { return layer.shy; }),
            quality: enumName(attempt(function () { return layer.quality; }), LayerQuality, QUALITY_NAMES),
            motion_blur: attempt(function () { return layer.motionBlur; }),
            adjustment: attempt(function () { return layer.adjustmentLayer; }),
            three_d: attempt(function () { return layer.threeDLayer; }),
            collapse: attempt(function () { return layer.collapseTransformation; }),
            frame_blending: enumName(attempt(function () { return layer.frameBlendingType; }),
                FrameBlendingType, FRAME_BLEND_NAMES),
            guide: attempt(function () { return layer.guideLayer; }),
            effects_active: attempt(function () { return layer.effectsActive; })
        };
        node.markers = markersOf(attempt(function () { return layer.property("ADBE Marker"); }));
        // The layer is itself a property group: walking it yields the transform
        // group, ADBE Mask Parade, ADBE Effect Parade, text, shape contents,
        // camera/light options, ADBE Time Remapping and layer styles at once.
        node.properties = walkGroup(layer, "");
        here.layer = "";
        return node;
    }

    // --- comps ------------------------------------------------------------
    function compOf(comp) {
        // here.comp names the comp in report rows only. The comp's own name is
        // not written into comps[]: a comp is also an item, and the name lives
        // there once, joined by id.
        here.comp = attempt(function () { return comp.name; }) || "";
        var node = {
            id: attempt(function () { return comp.id; }),
            width: attempt(function () { return comp.width; }),
            height: attempt(function () { return comp.height; }),
            par: attempt(function () { return comp.pixelAspect; }),
            fps: attempt(function () { return comp.frameRate; }),
            duration: attempt(function () { return comp.duration; }),
            start: attempt(function () { return comp.displayStartTime; }),
            bg_colour: attempt(function () { return numArray(comp.bgColor); }),
            motion_blur: {
                enabled: attempt(function () { return comp.motionBlur; }),
                shutter_angle: attempt(function () { return comp.shutterAngle; }),
                shutter_phase: attempt(function () { return comp.shutterPhase; }),
                samples: attempt(function () { return comp.motionBlurSamplesPerFrame; }),
                adaptive_limit: attempt(function () { return comp.motionBlurAdaptiveSampleLimit; })
            },
            renderer: attempt(function () { return comp.renderer; }),
            preserve_nested_fps: attempt(function () { return comp.preserveNestedFrameRate; }),
            preserve_nested_resolution: attempt(function () { return comp.preserveNestedResolution; }),
            markers: markersOf(attempt(function () { return comp.markerProperty; })),
            layers: []
        };
        var count = attempt(function () { return comp.numLayers; }) || 0;
        // Stacking order: index 1 is the top layer.
        for (var i = 1; i <= count; i++) {
            var layer = attempt(function () { return comp.layer(i); });
            if (!layer) continue;
            node.layers.push(layerOf(layer));
        }
        here.comp = "";
        return node;
    }

    // --- the walk ---------------------------------------------------------
    function walkProject() {
        var project = app.project;
        // Project-wide colour settings: docs/11 section 3 flags comps that relied on
        // non-linear 8-bpc blending, and nothing downstream can recover the
        // bit depth or the working space from the items alone.
        var capture = {
            project: {
                bits_per_channel: attempt(function () { return project.bitsPerChannel; }),
                working_space: attempt(function () { return project.workingSpace; }),
                linear_blending: attempt(function () { return project.linearBlending; }),
                linearize_working_space: attempt(function () { return project.linearizeWorkingSpace; }),
                expression_engine: attempt(function () { return project.expressionEngine; })
            },
            items: [],
            comps: []
        };
        var count = attempt(function () { return project.numItems; }) || 0;
        var comps = [];
        var i, item;
        // AE collections are 1-based.
        for (i = 1; i <= count; i++) {
            item = attempt(function () { return project.item(i); });
            if (!item) continue;
            capture.items.push(itemOf(item));
            if (item instanceof CompItem) comps.push(item);
        }
        for (i = 0; i < comps.length; i++) capture.comps.push(compOf(comps[i]));
        return capture;
    }

    function manifest() {
        return {
            format: "lumit-ae-bundle",
            version: BUNDLE_VERSION,
            ae_version: attempt(function () { return app.version; }),
            bridge_version: BRIDGE_VERSION,
            exported: today()
        };
    }

    // Writes manifest.json, capture.json and report.json into bundleFolder,
    // creating it if needed. Returns a small summary for the caller's alert.
    // No footage is collected in v1: paths only (docs/11 section 2.2 item 14).
    function exportBundle(bundleFolder) {
        report = { unreadables: [] };
        here = { comp: "", layer: "" };
        if (!bundleFolder.exists && !bundleFolder.create()) {
            throw new Error("could not create " + bundleFolder.fsName);
        }
        var capture = walkProject();
        writeJson(new File(bundleFolder.fsName + "/manifest.json"), manifest());
        writeJson(new File(bundleFolder.fsName + "/capture.json"), capture);
        writeJson(new File(bundleFolder.fsName + "/report.json"), report);
        return {
            folder: bundleFolder,
            items: capture.items.length,
            comps: capture.comps.length,
            unreadables: report.unreadables.length
        };
    }

    function run() {
        var project = app.project;
        if (!project) {
            alert("Open a project first, then run the Lumit Bridge.");
            return;
        }
        var base = project.file ? project.file.parent : Folder.desktop;
        Folder.current = base;
        var dest = Folder.selectDialog("Choose where the Lumit bundle folder goes", base);
        if (!dest) return;
        var stem = project.file
            ? String(project.file.name).replace(/\.aepx?$/i, "")
            : "untitled";
        var summary = exportBundle(new Folder(dest.fsName + "/" + stem + ".lum-bundle"));
        alert("Lumit bundle written:\n" + summary.folder.fsName +
            "\n\nItems: " + summary.items +
            "\nComps: " + summary.comps +
            "\nUnreadable properties: " + summary.unreadables +
            (summary.unreadables > 0 ? "\n(listed in report.json - this is expected for Curves and friends)" : ""));
    }

    return { exportBundle: exportBundle, run: run, version: BRIDGE_VERSION };
})();

// Publish on the global object explicitly rather than relying on a top-level
// var landing there: $.evalFile's scoping is the one thing make-fixture.jsx
// depends on and neither script can test.
$.global.LumitBridge = LumitBridge;

// Run the dialog only when this file is the script being run. make-fixture.jsx
// sets the flag on $.global before $.evalFile, and calls exportBundle itself.
if (!$.global.LUMIT_BRIDGE_EMBED) {
    LumitBridge.run();
}
