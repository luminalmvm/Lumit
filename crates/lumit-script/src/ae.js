// After Effects' scripting objects over Lumit's commands, so a .jsx written
// for After Effects runs as it is. Run once, after lumit.js and before the
// script.
//
// After Effects adds keys one at a time where Lumit's layer.keys sets a
// property's whole animation, so the keys a script makes are kept here and
// written once, when the script ends.
(function (N, scriptPath, windows) {
    var lumit = globalThis.lumit;
    var command = lumit.command;
    var D = Math.PI / 180;

    function unsupported(what) {
        throw new Error(what + ' is not supported in Lumit.');
    }

    var noted = {};
    function note(text) {
        if (!noted[text]) {
            noted[text] = true;
            N.log(text);
        }
    }

    // Names a script reads without meaning to, which answer undefined as
    // they would on any object.
    var quiet = { toJSON: 1, then: 1, inspect: 1 };

    // Asking one of these objects for something it does not have throws,
    // naming it, rather than quietly answering undefined.
    function guard(target, label) {
        return new Proxy(target, {
            get: function (t, k, r) {
                if (typeof k === 'string') {
                    if (t._at && /^\d+$/.test(k)) return t._at(Number(k));
                    if (!(k in t) && !quiet[k]) unsupported(label + '.' + k);
                }
                return Reflect.get(t, k, r);
            },
            set: function (t, k, v, r) {
                if (typeof k === 'string' && !(k in t)) unsupported(label + '.' + k);
                return Reflect.set(t, k, v, r);
            },
        });
    }

    function enumOf(label, names, first) {
        var e = {};
        names.forEach(function (name, n) { e[name] = first + n; });
        return guard(Object.freeze(e), label);
    }

    var AutoOrientType = enumOf('AutoOrientType',
        ['NO_AUTO_ORIENT', 'ALONG_PATH', 'CAMERA_OR_POINT_OF_INTEREST', 'CHARACTERS_TOWARD_CAMERA'], 4212);
    var KeyframeInterpolationType = enumOf('KeyframeInterpolationType', ['LINEAR', 'BEZIER', 'HOLD'], 6612);
    var ImportAsType = enumOf('ImportAsType', ['COMP_CROPPED_LAYERS', 'FOOTAGE', 'COMP', 'PROJECT'], 3812);

    // Seconds as After Effects gives them, from Lumit's exact times.
    function seconds(t) {
        return t.num / t.den;
    }

    // Lumit's commands may be called straight from the same script, so they
    // see every key made so far.
    (function wrap(o) {
        Object.keys(o).forEach(function (k) {
            var v = o[k];
            if (typeof v === 'function') {
                o[k] = function () { finish(); return v.apply(this, arguments); };
            } else if (v && typeof v === 'object' && k !== 'args') {
                wrap(v);
            }
        });
    })(lumit);

    // ---- Files and folders ----

    // Native paths, but After Effects' /c/folder form is read too.
    function nativePath(path) {
        var p = String(path);
        var drive = /^\/([a-zA-Z])(\/|$)/.exec(p);
        if (windows && drive) p = drive[1].toUpperCase() + ':' + p.slice(2);
        return p;
    }

    function parts(path) {
        return path.replace(/\\/g, '/').replace(/\/+$/, '');
    }

    function Entry(path, kind) {
        this._path = nativePath(path);
        this._kind = kind;
    }
    Object.defineProperties(Entry.prototype, {
        name: { get: function () { var p = parts(this._path); return p.slice(p.lastIndexOf('/') + 1); } },
        displayName: { get: function () { return this.name; } },
        fsName: { get: function () { return windows ? this._path.replace(/\//g, '\\') : this._path; } },
        fullName: { get: function () { return parts(this._path); } },
        absoluteURI: { get: function () { return parts(this._path); } },
        path: { get: function () { var p = parts(this._path); return p.slice(0, p.lastIndexOf('/')); } },
        parent: {
            get: function () {
                var p = this.path;
                return p ? new Folder(p) : null;
            },
        },
        exists: { get: function () { return JSON.parse(N.exists(this._path)).exists; } },
    });
    Entry.prototype.toString = function () { return this.fullName; };

    function File(path) {
        if (!(this instanceof File)) return new File(path);
        Entry.call(this, path === undefined ? '' : path, 'File');
        this._text = null;
        this._read = 0;
        this._at = undefined;
        this.encoding = 'UTF-8';
        return guard(this, 'File');
    }
    File.prototype = Object.create(Entry.prototype);
    File.prototype.constructor = File;
    File.prototype.open = function (mode) {
        if (mode && String(mode).charAt(0) !== 'r') unsupported('File.open("' + mode + '")');
        this._text = JSON.parse(N.readText(this._path));
        this._read = 0;
        return true;
    };
    File.prototype.read = function () {
        if (this._text === null) this.open('r');
        var rest = this._text.slice(this._read);
        this._read = this._text.length;
        return rest;
    };
    File.prototype.readln = function () {
        if (this._text === null) this.open('r');
        var end = this._text.indexOf('\n', this._read);
        if (end < 0) end = this._text.length;
        var line = this._text.slice(this._read, end).replace(/\r$/, '');
        this._read = end + 1;
        return line;
    };
    Object.defineProperty(File.prototype, 'eof', {
        get: function () { return this._text === null || this._read >= this._text.length; },
    });
    File.prototype.close = function () { this._text = null; return true; };
    File.openDialog = function (prompt, filter) {
        var extensions = [];
        String(filter || '').replace(/\*\.(\w+)/g, function (_, e) { extensions.push(e); });
        var path = lumit.chooseFile(prompt || '', extensions);
        return path === null ? null : new File(path);
    };

    function Folder(path) {
        if (!(this instanceof Folder)) return new Folder(path);
        Entry.call(this, path === undefined ? '' : path, 'Folder');
        this._at = undefined;
        return guard(this, 'Folder');
    }
    Folder.prototype = Object.create(Entry.prototype);
    Folder.prototype.constructor = Folder;
    Folder.prototype.getFiles = function (mask) {
        var test = function () { return true; };
        if (typeof mask === 'function') {
            test = mask;
        } else if (mask) {
            var globs = String(mask).split(/[;,]/).map(function (g) {
                var re = g.trim().replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.');
                return new RegExp('^' + re + '$', 'i');
            });
            test = function (entry) { return globs.some(function (re) { return re.test(entry.name); }); };
        }
        var base = parts(this._path);
        return JSON.parse(N.list(this._path)).map(function (e) {
            var path = base + '/' + e.name;
            return e.folder ? new Folder(path) : new File(path);
        }).filter(function (entry) { return test(entry); });
    };

    // ---- The project and its items ----

    var items = {};

    function itemInfo(id) {
        var found = command('item.list').items.filter(function (i) { return i.id === id; })[0];
        if (!found) throw new Error('That item is no longer in the project.');
        return found;
    }

    function itemFor(info) {
        if (!info) return null;
        var known = items[info.id];
        if (known) return known;
        var made;
        if (info.kind === 'comp') made = new CompItem(info.id);
        else if (info.kind === 'folder') made = new FolderItem(info.id);
        else made = new FootageItem(info.id);
        items[info.id] = made;
        return made;
    }

    function Item(id, label) {
        this._id = id;
        this._at = undefined;
        this._label = label;
    }
    Object.defineProperties(Item.prototype, {
        id: { get: function () { return this._id; } },
        name: {
            get: function () { return itemInfo(this._id).name; },
            set: function (v) { command('item.set', { item: this._id, name: String(v) }); },
        },
        parentFolder: {
            get: function () {
                var folder = itemInfo(this._id).folder;
                return folder ? itemFor({ id: folder, kind: 'folder' }) : rootFolder;
            },
            set: function (f) {
                if (!(f instanceof FolderItem)) throw new Error('parentFolder takes a folder.');
                command('item.set', { item: this._id, folder: f._id });
            },
        },
        comment: {
            get: function () { return ''; },
            set: function () { note('Item comments are not kept, as Lumit items have no comment.'); },
        },
    });
    Item.prototype.remove = function () { unsupported(this._label + '.remove()'); };

    function ItemCollection(folder) {
        this._folder = folder;
        this._at = function (n) { return this._list()[n - 1]; };
        return guard(this, 'ItemCollection');
    }
    // The whole project's items when `folder` is undefined, the top of the
    // panel's when it is null.
    ItemCollection.prototype._list = function () {
        var folder = this._folder;
        return command('item.list').items.filter(function (i) {
            return folder === undefined || i.folder === folder;
        }).map(itemFor);
    };
    Object.defineProperty(ItemCollection.prototype, 'length', {
        get: function () { return this._list().length; },
    });
    ItemCollection.prototype.addFolder = function (name) {
        var made = command('folder.new', { name: String(name), parent: this._folder || undefined });
        return itemFor(made);
    };
    ItemCollection.prototype.addComp = function (name, width, height, pixelAspect, duration, frameRate) {
        var rate = JSON.parse(N.rateOf(Number(frameRate)));
        if (Math.abs(Number(pixelAspect) - 1) > 1e-3) {
            note('Lumit comps have square pixels, so a pixel aspect of ' + pixelAspect + ' was left out.');
        }
        var made = command('comp.new', {
            name: String(name),
            width: Math.round(width),
            height: Math.round(height),
            frame_rate: rate,
            duration: N.frameOn(Number(duration), rate.num, rate.den),
            // Where After Effects puts it, rather than in a Compositions folder.
            folder: this._folder || null,
        });
        return itemFor({ id: made.id, kind: 'comp' });
    };

    function FolderItem(id) {
        Item.call(this, id, 'FolderItem');
        return guard(this, 'FolderItem');
    }
    FolderItem.prototype = Object.create(Item.prototype);
    FolderItem.prototype.constructor = FolderItem;
    Object.defineProperties(FolderItem.prototype, {
        typeName: { value: 'Folder' },
        items: { get: function () { return new ItemCollection(this._id); } },
        numItems: { get: function () { return this.items.length; } },
    });
    FolderItem.prototype.item = function (n) { return this.items[n]; };

    // The top of the project panel, which After Effects calls a folder too.
    var rootFolder = (function () {
        var root = Object.create(FolderItem.prototype);
        root._id = null;
        root._at = undefined;
        Object.defineProperty(root, 'name', { value: 'Root' });
        Object.defineProperty(root, 'parentFolder', { value: null });
        return guard(root, 'FolderItem');
    })();

    function FootageSource(item) {
        this._item = item;
        this._at = undefined;
        return guard(this, 'FootageSource');
    }
    Object.defineProperties(FootageSource.prototype, {
        conformFrameRate: {
            get: function () {
                var r = itemInfo(this._item._id).frame_rate;
                return r ? r.num / r.den : 0;
            },
            set: function (fps) {
                var id = this._item._id;
                if (Number(fps) > 0) {
                    command('item.set', { item: id, frame_rate: JSON.parse(N.rateOf(Number(fps))) });
                } else if (!itemInfo(id).sequence) {
                    // Nought is a video's own rate. A run of stills has none,
                    // so it keeps the rate it has.
                    command('item.set', { item: id, frame_rate: null });
                }
            },
        },
        frameRate: { get: function () { return this.conformFrameRate; } },
        file: { get: function () { return this._item.file; } },
        isStill: { get: function () { return false; } },
    });

    function FootageItem(id) {
        Item.call(this, id, 'FootageItem');
        this._duration = null;
        return guard(this, 'FootageItem');
    }
    FootageItem.prototype = Object.create(Item.prototype);
    FootageItem.prototype.constructor = FootageItem;
    Object.defineProperties(FootageItem.prototype, {
        typeName: { value: 'Footage' },
        mainSource: { get: function () { return new FootageSource(this); } },
        file: { get: function () { return new File(itemInfo(this._id).path); } },
        frameRate: { get: function () { return this.mainSource.conformFrameRate; } },
        // Lumit reads a file's length when it places it, so this is known once
        // the footage has a layer.
        duration: {
            get: function () {
                if (this._duration === null) unsupported('FootageItem.duration before the footage is in a comp');
                return this._duration;
            },
        },
    });

    function CompItem(id) {
        Item.call(this, id, 'CompItem');
        this._rate = null;
        return guard(this, 'CompItem');
    }
    CompItem.prototype = Object.create(Item.prototype);
    CompItem.prototype.constructor = CompItem;
    CompItem.prototype._get = function () { return command('comp.get', { comp: this._id }); };
    CompItem.prototype._set = function (args) {
        args.comp = this._id;
        var answer = command('comp.set', args);
        this._rate = answer.frame_rate;
        return answer;
    };
    CompItem.prototype._fps = function () {
        if (!this._rate) this._rate = this._get().frame_rate;
        return this._rate;
    };
    // The nearest frame of this comp to `t` seconds.
    CompItem.prototype._frame = function (t) {
        var r = this._fps();
        return N.frameOn(Number(t), r.num, r.den);
    };
    Object.defineProperties(CompItem.prototype, {
        typeName: { value: 'Composition' },
        width: {
            get: function () { return this._get().width; },
            set: function (v) { this._set({ width: Math.round(v) }); },
        },
        height: {
            get: function () { return this._get().height; },
            set: function (v) { this._set({ height: Math.round(v) }); },
        },
        pixelAspect: {
            get: function () { return 1; },
            set: function (v) {
                if (Math.abs(Number(v) - 1) > 1e-3) note('Lumit comps have square pixels, so a pixel aspect of ' + v + ' was left out.');
            },
        },
        frameRate: {
            get: function () { var r = this._fps(); return r.num / r.den; },
            set: function (v) { this._set({ frame_rate: JSON.parse(N.rateOf(Number(v))) }); },
        },
        frameDuration: { get: function () { var r = this._fps(); return r.den / r.num; } },
        duration: {
            get: function () { return seconds(this._get().duration); },
            set: function (v) { this._set({ duration: this._frame(v) }); },
        },
        layers: { get: function () { return new LayerCollection(this); } },
        numLayers: { get: function () { return this._get().layers.length; } },
    });
    CompItem.prototype.layer = function (which) {
        return typeof which === 'string' ? this.layers.byName(which) : this.layers.byIndex(which);
    };
    CompItem.prototype.openInViewer = function () {
        command('comp.open', { comp: this._id });
        return null;
    };

    // ---- Layers ----

    var layers = {};
    var dirty = [];

    function layerFor(comp, id) {
        return layers[id] || (layers[id] = new Layer(comp, id));
    }

    function LayerCollection(comp) {
        this._comp = comp;
        this._at = function (n) { return this.byIndex(n); };
        return guard(this, 'LayerCollection');
    }
    LayerCollection.prototype._list = function () { return this._comp._get().layers; };
    LayerCollection.prototype.byIndex = function (n) {
        var l = this._list()[n - 1];
        if (!l) throw new Error('There is no layer ' + n + ' in ' + this._comp.name + '.');
        return layerFor(this._comp, l.id);
    };
    LayerCollection.prototype.byName = function (name) {
        var l = this._list().filter(function (x) { return x.name === name; })[0];
        return l ? layerFor(this._comp, l.id) : null;
    };
    Object.defineProperty(LayerCollection.prototype, 'length', {
        get: function () { return this._list().length; },
    });
    LayerCollection.prototype._add = function (kind, name, item) {
        var made = command('layer.add', { comp: this._comp._id, kind: kind, name: name, item: item });
        return layerFor(this._comp, made.id);
    };
    LayerCollection.prototype.addCamera = function (name, centerPoint) {
        var layer = this._add('camera', String(name));
        // After Effects' new camera aims at its point of interest, here the
        // point it was given, from a zoom's distance in front.
        command('layer.set', { comp: this._comp._id, layer: layer._id, two_node: true });
        if (centerPoint) {
            var at = [Number(centerPoint[0]), Number(centerPoint[1])];
            var z = layer._info().properties.position[2];
            command('layer.keys', { comp: this._comp._id, layer: layer._id, property: 'position', value: [at[0], at[1], z] });
            command('layer.keys', { comp: this._comp._id, layer: layer._id, property: 'point_of_interest', value: [at[0], at[1], 0] });
        }
        return layer;
    };
    LayerCollection.prototype.addNull = function (duration) {
        var layer = this._add('null');
        // After Effects' null is 100 pixels square with its anchor in the
        // middle, and a child's position is measured from its corner.
        command('layer.keys', { comp: this._comp._id, layer: layer._id, property: 'anchor', value: [50, 50] });
        if (duration !== undefined) layer.outPoint = Number(duration);
        return layer;
    };
    LayerCollection.prototype.add = function (item, duration) {
        if (!(item instanceof FootageItem)) unsupported('layers.add of a ' + (item && item.typeName));
        var layer = this._add('footage', undefined, item._id);
        var info = layer._info();
        item._duration = seconds(info.out) - seconds(info.in);
        if (duration !== undefined) layer.outPoint = layer.inPoint + Number(duration);
        return layer;
    };

    function Layer(comp, id) {
        this._comp = comp;
        this._id = id;
        this._at = undefined;
        this._props = {};
        this._removed = false;
        this._kind = undefined;
        return guard(this, 'Layer');
    }
    Layer.prototype._info = function () { return command('layer.get', { comp: this._comp._id, layer: this._id }); };
    Layer.prototype._set = function (args) {
        args.comp = this._comp._id;
        args.layer = this._id;
        return command('layer.set', args);
    };
    function switchProp(name) {
        return {
            get: function () { return this._info().switches[name]; },
            set: function (v) { var s = {}; s[name] = !!v; this._set({ switches: s }); },
        };
    }
    Object.defineProperties(Layer.prototype, {
        name: {
            get: function () { return this._info().name; },
            set: function (v) { this._set({ name: String(v) }); },
        },
        id: { get: function () { return this._id; } },
        index: { get: function () { return this._info().index + 1; } },
        containingComp: { get: function () { return this._comp; } },
        enabled: switchProp('visible'),
        audioEnabled: switchProp('audible'),
        solo: switchProp('solo'),
        locked: switchProp('locked'),
        shy: switchProp('shy'),
        guideLayer: switchProp('guide'),
        motionBlur: switchProp('motion_blur'),
        effectsActive: switchProp('fx'),
        threeDLayer: {
            get: function () { return this._info().three_d; },
            set: function (v) { this._set({ three_d: !!v }); },
        },
        startTime: {
            get: function () { return seconds(this._info().start_offset); },
            set: function (v) {
                // After Effects moves the whole layer, so its In and Out go
                // with it, and so do the keys made or read so far.
                var info = this._info(), comp = this._comp, props = this._props;
                var shift = Number(v) - seconds(info.start_offset);
                var now = this._set({
                    start_offset: comp._frame(v),
                    in: comp._frame(seconds(info.in) + shift),
                    out: comp._frame(seconds(info.out) + shift),
                });
                var moved = seconds(now.start_offset) - seconds(info.start_offset);
                Object.keys(props).forEach(function (n) {
                    (props[n]._keys || []).forEach(function (k) {
                        k.t += moved;
                        k.f = comp._frame(k.t);
                    });
                });
            },
        },
        inPoint: {
            get: function () { return seconds(this._info().in); },
            set: function (v) { this._set({ in: this._comp._frame(v) }); },
        },
        outPoint: {
            get: function () { return seconds(this._info().out); },
            set: function (v) { this._set({ out: this._comp._frame(v) }); },
        },
        parent: {
            get: function () {
                var p = this._info().parent;
                return p ? layerFor(this._comp, p) : null;
            },
            set: function (p) { this._parent(p, false); },
        },
        autoOrient: {
            get: function () {
                return this._info().two_node ? AutoOrientType.CAMERA_OR_POINT_OF_INTEREST : AutoOrientType.NO_AUTO_ORIENT;
            },
            set: function (v) {
                var info = this._info();
                if (info.kind === 'camera' && (v === AutoOrientType.NO_AUTO_ORIENT || v === AutoOrientType.CAMERA_OR_POINT_OF_INTEREST)) {
                    this._set({ two_node: v === AutoOrientType.CAMERA_OR_POINT_OF_INTEREST });
                } else if (v !== AutoOrientType.NO_AUTO_ORIENT) {
                    unsupported('autoOrient other than NO_AUTO_ORIENT on a ' + info.kind + ' layer');
                }
            },
        },
        transform: { get: function () { return this.property('ADBE Transform Group'); } },
        cameraOption: { get: function () { return this.property('ADBE Camera Options Group'); } },
    });
    // After Effects keeps the layer where it is unless told to jump, which
    // changes its transform. So the keys made so far go in first, and what
    // was read of the layer is read again.
    Layer.prototype._parent = function (p, jump) {
        finish();
        this._set({ parent: p ? p._id : null, jump: jump });
        Object.keys(this._props).forEach(function (n) { this._props[n]._keys = null; }, this);
    };
    Layer.prototype.setParentWithJump = function (p) { this._parent(p, true); };
    Layer.prototype._move = function (index) { this._set({ index: index }); };
    Layer.prototype.moveToBeginning = function () { this._move(0); };
    Layer.prototype.moveToEnd = function () { this._move(this._comp.numLayers - 1); };
    // Lumit's place is counted once the layer is lifted out of the stack.
    Layer.prototype._beside = function (other, after) {
        var at = other.index - 1, me = this.index - 1;
        this._move(at - (me < at ? 1 : 0) + (after ? 1 : 0));
    };
    Layer.prototype.moveBefore = function (other) { this._beside(other, false); };
    Layer.prototype.moveAfter = function (other) { this._beside(other, true); };
    Layer.prototype.remove = function () {
        command('layer.delete', { comp: this._comp._id, layer: this._id });
        this._removed = true;
    };

    var GROUPS = {
        'ADBE Transform Group': 'transform', 'Transform': 'transform',
        'ADBE Camera Options Group': 'camera', 'Camera Options': 'camera',
    };
    var TRANSFORM = {
        'ADBE Anchor Point': 'anchor', 'Anchor Point': 'anchor', 'Point of Interest': 'anchor',
        'ADBE Position': 'position', 'Position': 'position',
        'ADBE Scale': 'scale', 'Scale': 'scale',
        'ADBE Orientation': 'orientation', 'Orientation': 'orientation',
        'ADBE Rotate X': 'rotation_x', 'X Rotation': 'rotation_x',
        'ADBE Rotate Y': 'rotation_y', 'Y Rotation': 'rotation_y',
        'ADBE Rotate Z': 'rotation', 'Z Rotation': 'rotation', 'Rotation': 'rotation',
        'ADBE Opacity': 'opacity', 'Opacity': 'opacity',
    };
    var CAMERA = {
        'ADBE Camera Zoom': 'zoom', 'Zoom': 'zoom',
        'ADBE Camera Focus Distance': 'focus_distance', 'Focus Distance': 'focus_distance',
        'ADBE Camera Aperture': 'aperture', 'Aperture': 'aperture',
    };

    function PropertyGroup(layer, table, label) {
        this._layer = layer;
        this._table = table;
        this._at = undefined;
        return guard(this, label);
    }
    PropertyGroup.prototype.property = function (name) {
        var own = this._table[name];
        if (!own) unsupported('property("' + name + '")');
        return this._layer._property(own);
    };
    // The short names, as transform.position.
    [['anchorPoint', 'anchor'], ['pointOfInterest', 'anchor'], ['position', 'position'], ['scale', 'scale'],
        ['orientation', 'orientation'], ['xRotation', 'rotation_x'], ['yRotation', 'rotation_y'],
        ['zRotation', 'rotation'], ['rotation', 'rotation'], ['opacity', 'opacity'], ['zoom', 'zoom'],
        ['focusDistance', 'focus_distance'], ['aperture', 'aperture']].forEach(function (pair) {
        Object.defineProperty(PropertyGroup.prototype, pair[0], {
            get: function () { return this._layer._property(pair[1]); },
        });
    });

    Layer.prototype.property = function (name) {
        var group = GROUPS[name];
        if (group === 'transform') return new PropertyGroup(this, TRANSFORM, 'Transform');
        if (group === 'camera') return new PropertyGroup(this, CAMERA, 'Camera Options');
        // After Effects finds a transform property from the layer too.
        var own = TRANSFORM[name] || CAMERA[name];
        if (own) return this._property(own);
        unsupported('property("' + name + '")');
    };
    Layer.prototype._property = function (name) {
        // A camera's anchor point is its point of interest.
        if (name === 'anchor' && this._isCamera()) name = 'point_of_interest';
        return this._props[name] || (this._props[name] = new Property(this, name));
    };
    Layer.prototype._isCamera = function () {
        if (this._kind === undefined) this._kind = this._info().kind;
        return this._kind === 'camera';
    };

    // How many values each of Lumit's properties takes, and what After
    // Effects' third one must be where Lumit has only two.
    var AXES = { anchor: [2, 0], scale: [2, 100], position: [3], point_of_interest: [3], orientation: [3] };

    // After Effects' own names, for messages and Property.name.
    var LABELS = {
        anchor: 'Anchor Point', point_of_interest: 'Point of Interest', position: 'Position', scale: 'Scale',
        orientation: 'Orientation', rotation_x: 'X Rotation', rotation_y: 'Y Rotation', rotation: 'Z Rotation',
        opacity: 'Opacity', zoom: 'Zoom', focus_distance: 'Focus Distance', aperture: 'Aperture',
    };

    // A value as After Effects shows it.
    function aeValue(name, v) {
        var axes = AXES[name];
        if (!axes) return Number(v instanceof Array ? v[0] : v);
        v = Array.prototype.slice.call(v).map(Number);
        if (axes.length > 1 && v.length === 2) v.push(axes[1]);
        if (v.length === 2) v.push(0);
        return v;
    }

    // A value as Lumit takes it.
    function lumitValue(name, v) {
        var axes = AXES[name];
        if (!axes || axes.length === 1) return v;
        if (v[2] !== axes[1]) {
            throw new Error(LABELS[name] + "'s third value has to be " + axes[1] + ' in Lumit, which has no Z there.');
        }
        return v.slice(0, 2);
    }

    function Key(t, v, frame) {
        this.t = t;
        this.v = v;
        this.f = frame;
        this.hold = false;
    }

    function Property(layer, name) {
        this._layer = layer;
        this._name = name;
        this._keys = null;
        this._value = null;
        this._changed = false;
        this._at = undefined;
        return guard(this, 'Property');
    }
    // What the document holds, read the first time the script touches it.
    Property.prototype._load = function () {
        if (this._keys !== null) return;
        var name = this._name;
        this._keys = [];
        if (name === 'orientation') {
            this._value = [0, 0, 0];
            return;
        }
        var info = this._layer._info(), held = info.properties[name];
        if (held === undefined) throw new Error(LABELS[name] + ' is not on this layer.');
        if (TURNS.indexOf(name) >= 0 && (info.kind === 'camera' || info.three_d)) {
            loadTurns(this._layer, info, this);
            return;
        }
        var channels = AXES[name] ? held : [held];
        function make(values) { return aeValue(name, AXES[name] ? values : values[0]); }
        this._value = make(channels.map(function (c) { return typeof c === 'number' ? c : 0; }));
        var times = channelTimes(channels);
        for (var i = 0; i < times.length; i++) {
            var t = times[i];
            var k = new Key(t, make(channels.map(function (c) { return channelAt(c, t); })), this._layer._comp._frame(t));
            k.hold = channels.every(function (c) { return heldAt(c, t); });
            this._keys.push(k);
        }
    };

    // Lumit keeps each axis apart, with keys of its own, where After Effects'
    // keys carry every axis. So there is a key wherever any axis has one.
    function channelTimes(channels) {
        var times = [];
        channels.forEach(function (c) {
            if (c instanceof Array) c.forEach(function (k) { times.push(seconds(k.time)); });
        });
        times.sort(function (a, b) { return a - b; });
        return times.filter(function (t, n) { return n === 0 || t !== times[n - 1]; });
    }

    // One axis at `t` seconds, between its keys in a straight line or held.
    function channelAt(c, t) {
        if (typeof c === 'number') return c;
        if (!(c instanceof Array) || !c.length) return 0;
        if (t <= seconds(c[0].time)) return c[0].value;
        for (var i = 1; i < c.length; i++) {
            var b = seconds(c[i].time);
            if (t < b) {
                var a = c[i - 1], from = seconds(a.time);
                if (a.hold) return a.value;
                return a.value + (c[i].value - a.value) * (t - from) / (b - from);
            }
        }
        return c[c.length - 1].value;
    }

    // Whether an axis holds after `t`. One with no keys never moves.
    function heldAt(c, t) {
        if (!(c instanceof Array)) return true;
        var held = false;
        c.forEach(function (k) { if (seconds(k.time) <= t) held = !!k.hold; });
        return held;
    }
    Property.prototype._dirty = function () {
        if (dirty.indexOf(this._layer) < 0) dirty.push(this._layer);
        this._changed = true;
    };
    Property.prototype._key = function (k) {
        this._load();
        var key = this._keys[k - 1];
        if (!key) throw new Error('There is no key ' + k + ' on ' + this.name + '.');
        return key;
    };
    // The value After Effects was given, checked now so a mistake is reported
    // on the script's line rather than when the keys are written.
    Property.prototype._checked = function (v) {
        var value = aeValue(this._name, v);
        lumitValue(this._name, value);
        return value;
    };
    // Put a key at `t`, over any already on that frame, and answer its index.
    Property.prototype._place = function (t, v) {
        this._load();
        var frame = this._layer._comp._frame(t), keys = this._keys;
        var key = new Key(Number(t), this._checked(v), frame);
        if (!keys.length || keys[keys.length - 1].f < frame) {
            keys.push(key);
            return keys.length;
        }
        for (var i = 0; i < keys.length; i++) {
            var at = keys[i].f;
            if (at === frame) {
                key.hold = keys[i].hold;
                keys[i] = key;
                return i + 1;
            }
            if (at > frame) {
                keys.splice(i, 0, key);
                return i + 1;
            }
        }
        keys.push(key);
        return keys.length;
    };
    Object.defineProperties(Property.prototype, {
        name: { get: function () { return LABELS[this._name]; } },
        numKeys: { get: function () { this._load(); return this._keys.length; } },
        isTimeVarying: { get: function () { return this.numKeys > 0; } },
        canVaryOverTime: { value: true },
        value: { get: function () { return this.valueAtTime(0); } },
        expression: {
            get: function () { return ''; },
            set: function () { unsupported('Property.expression'); },
        },
    });
    Property.prototype.setValue = function (v) {
        this._load();
        if (this._keys.length) throw new Error(this.name + ' has keys, so its value is set with setValueAtTime.');
        this._value = this._checked(v);
        this._dirty();
    };
    Property.prototype.setValueAtTime = function (t, v) {
        this._place(t, v);
        this._dirty();
    };
    Property.prototype.setValuesAtTimes = function (times, values) {
        if (times.length !== values.length) throw new Error('setValuesAtTimes needs a value for each time.');
        for (var i = 0; i < times.length; i++) this._place(times[i], values[i]);
        this._dirty();
    };
    Property.prototype.addKey = function (t) {
        var at = this._place(t, this.valueAtTime(t));
        this._dirty();
        return at;
    };
    Property.prototype.removeKey = function (k) {
        var key = this._key(k);
        // With the last key gone the value stays what it was.
        if (this._keys.length === 1) this._value = key.v;
        this._keys.splice(k - 1, 1);
        this._dirty();
    };
    Property.prototype.keyTime = function (k) { return this._key(k).t; };
    Property.prototype.keyValue = function (k) { return this._key(k).v; };
    Property.prototype.nearestKeyIndex = function (t) {
        this._load();
        var best = 0;
        for (var i = 1; i < this._keys.length; i++) {
            if (Math.abs(this._keys[i].t - t) < Math.abs(this._keys[best].t - t)) best = i;
        }
        return this._keys.length ? best + 1 : 0;
    };
    Property.prototype.valueAtTime = function (t) {
        this._load();
        return valueOf(this, Number(t));
    };
    Property.prototype.setInterpolationTypeAtKey = function (k, inType, outType) {
        var key = this._key(k);
        var out = outType === undefined ? inType : outType;
        if (inType === KeyframeInterpolationType.BEZIER || out === KeyframeInterpolationType.BEZIER) {
            note('Bezier keys come in as linear ones.');
        }
        key.hold = out === KeyframeInterpolationType.HOLD;
        this._dirty();
    };
    Property.prototype.keyInInterpolationType = function (k) {
        this._key(k);
        return KeyframeInterpolationType.LINEAR;
    };
    Property.prototype.keyOutInterpolationType = function (k) {
        return this._key(k).hold ? KeyframeInterpolationType.HOLD : KeyframeInterpolationType.LINEAR;
    };
    // Lumit's keys run in straight lines at an even speed, which these calls
    // can only ask for or leave as it is.
    Property.prototype.setSpatialAutoBezierAtKey = function (k, on) {
        this._key(k);
        if (on) note('Lumit moves between keys in straight lines.');
    };
    Property.prototype.setSpatialContinuousAtKey = function (k) { this._key(k); };
    Property.prototype.setSpatialTangentsAtKey = function (k, inTangent, outTangent) {
        this._key(k);
        var curved = [inTangent, outTangent || inTangent].some(function (t) {
            return t && Array.prototype.some.call(t, function (x) { return Number(x) !== 0; });
        });
        if (curved) note('Lumit moves between keys in straight lines.');
    };
    ['setTemporalEaseAtKey', 'setTemporalContinuousAtKey', 'setTemporalAutoBezierAtKey', 'setRovingAtKey'].forEach(function (name) {
        Property.prototype[name] = function (k) {
            this._key(k);
            note('Ease on keys is not carried, so keys run at an even speed.');
        };
    });

    function valueOf(prop, t) {
        var keys = prop._keys;
        if (!keys.length) return prop._value;
        if (t <= keys[0].t) return keys[0].v;
        for (var i = 1; i < keys.length; i++) {
            if (t < keys[i].t) {
                var a = keys[i - 1], b = keys[i];
                if (a.hold) return a.v;
                var f = (t - a.t) / (b.t - a.t);
                if (a.v instanceof Array) return a.v.map(function (x, n) { return x + (b.v[n] - x) * f; });
                return a.v + (b.v - a.v) * f;
            }
        }
        return keys[keys.length - 1].v;
    }

    // ---- Writing the keys ----

    function write(layer, name, value, keys) {
        var args = { comp: layer._comp._id, layer: layer._id, property: name };
        if (keys && keys.length) {
            var out = [];
            keys.forEach(function (k) {
                var key = { time: layer._comp._frame(k.t), value: lumitValue(name, k.v), interp: k.hold ? 'hold' : 'linear' };
                // Two keys on one frame: the later wins, as in After Effects.
                if (out.length && out[out.length - 1].time === key.time) out[out.length - 1] = key;
                else out.push(key);
            });
            args.keys = out;
        } else {
            args.value = lumitValue(name, value);
        }
        command('layer.keys', args);
    }

    function rx(a) { var c = Math.cos(a * D), s = Math.sin(a * D); return [[1, 0, 0], [0, c, -s], [0, s, c]]; }
    function ry(a) { var c = Math.cos(a * D), s = Math.sin(a * D); return [[c, 0, s], [0, 1, 0], [-s, 0, c]]; }
    function rz(a) { var c = Math.cos(a * D), s = Math.sin(a * D); return [[c, -s, 0], [s, c, 0], [0, 0, 1]]; }
    function mul(a, b) {
        return a.map(function (row) {
            return [0, 1, 2].map(function (c) { return row[0] * b[0][c] + row[1] * b[1][c] + row[2] * b[2][c]; });
        });
    }

    // After Effects turns a 3D layer by Orientation, then X, Y and Z
    // Rotation, as Rx · Ry · Rz each. Lumit turns it by Ry · Rx · Rz, so the
    // same turn needs other angles whenever X and Y are both used.
    function lumitAngles(o, a, near) {
        var m = mul(mul(mul(rx(o[0]), ry(o[1])), rz(o[2])), mul(mul(rx(a[0]), ry(a[1])), rz(a[2])));
        // Lumit's own reading of a matrix (lumit_core::camera::euler_yxz),
        // with Y's sign right when X is a quarter turn.
        var sx = Math.max(-1, Math.min(1, -m[1][2]));
        var x = Math.asin(sx), cx = Math.sqrt(Math.max(0, 1 - sx * sx)), one;
        if (cx < 1e-9) one = [x / D, Math.atan2(-m[2][0], m[0][0]) / D, 0];
        else one = [x / D, Math.atan2(m[0][2], m[2][2]) / D, Math.atan2(m[1][0], m[1][1]) / D];
        return nearer(one, [180 - one[0], one[1] + 180, one[2] + 180], near);
    }

    // The same turn back in After Effects' order, from Lumit's angles `l`.
    function aeAngles(l, near) {
        var m = mul(mul(ry(l[1]), rx(l[0])), rz(l[2]));
        var sy = Math.max(-1, Math.min(1, m[0][2]));
        var y = Math.asin(sy), cy = Math.sqrt(Math.max(0, 1 - sy * sy)), one;
        if (cy < 1e-9) one = [Math.atan2(m[2][1], m[1][1]) / D, y / D, 0];
        else one = [Math.atan2(-m[1][2], m[2][2]) / D, y / D, Math.atan2(-m[0][1], m[0][0]) / D];
        return nearer(one, [one[0] + 180, 180 - one[1], one[2] + 180], near || l);
    }

    // Of two ways of writing one turn, whichever is nearer the last key,
    // wound on by whole turns so nothing spins between keys.
    function nearer(one, other, near) {
        function wound(v) {
            return v.map(function (x, n) {
                while (x - near[n] > 180) x -= 360;
                while (x - near[n] < -180) x += 360;
                return x;
            });
        }
        function gap(v) { return v.reduce(function (s, x, n) { return s + (x - near[n]) * (x - near[n]); }, 0); }
        one = wound(one);
        other = wound(other);
        return gap(other) < gap(one) ? other : one;
    }

    // A 3D layer's X, Y and Z Rotation as After Effects reads them, which
    // takes all three at once. `caller` is the one being read; the others
    // are filled too unless already read.
    var TURNS = ['rotation_x', 'rotation_y', 'rotation'];
    function loadTurns(layer, info, caller) {
        var channels = TURNS.map(function (n) { return info.properties[n]; });
        var still = channels.map(function (c) { return typeof c === 'number' ? c : 0; });
        still = aeAngles(still, still);
        var lists = [[], [], []], near = null;
        channelTimes(channels).forEach(function (t) {
            var turned = aeAngles(channels.map(function (c) { return channelAt(c, t); }), near);
            near = turned;
            var hold = channels.every(function (c) { return heldAt(c, t); });
            turned.forEach(function (v, n) {
                var k = new Key(t, v, layer._comp._frame(t));
                k.hold = hold;
                lists[n].push(k);
            });
        });
        TURNS.forEach(function (name, n) {
            var p = layer._property(name);
            if (p !== caller && p._keys !== null) return;
            p._keys = lists[n];
            p._value = still[n];
        });
    }

    function used(prop) {
        var all = prop._keys.length ? prop._keys.map(function (k) { return k.v; }) : [prop._value];
        return all.some(function (v) { return v instanceof Array ? v.some(Boolean) : v !== 0; });
    }

    function writeRotations(layer) {
        var p = {}, names = ['rotation_x', 'rotation_y', 'rotation', 'orientation'];
        names.forEach(function (n) { p[n] = layer._props[n]; });
        if (!names.some(function (n) { return p[n] && p[n]._changed; })) return;
        names.forEach(function (n) {
            if (!p[n]) p[n] = layer._property(n);
            p[n]._load();
        });
        var o = p.orientation;
        if (o._keys.length) unsupported('Keys on Orientation');
        var info = layer._info();
        var solid = info.kind === 'camera' || info.three_d;
        if (!solid || (!used(o) && !(used(p.rotation_x) && used(p.rotation_y)))) {
            if (used(o)) unsupported('Orientation on a 2D layer');
            ['rotation_x', 'rotation_y', 'rotation'].forEach(function (n) {
                if (p[n]._changed) write(layer, n, p[n]._value, p[n]._keys);
            });
            return;
        }
        var trio = [p.rotation_x, p.rotation_y, p.rotation];
        var times = [];
        trio.forEach(function (q) { q._keys.forEach(function (k) { times.push(k.t); }); });
        times.sort(function (a, b) { return a - b; });
        if (!times.length) {
            var still = trio.map(function (q) { return q._value; });
            var turned = lumitAngles(o._value, still, still);
            trio.forEach(function (q, n) { write(layer, q._name, turned[n]); });
            return;
        }
        note('X, Y and Z Rotation were turned into Lumit\'s order, which matches After Effects on every key.');
        var lists = [[], [], []], near = null, last = null;
        times.forEach(function (t) {
            var frame = layer._comp._frame(t);
            if (frame === last) return;
            last = frame;
            var angles = trio.map(function (q) { return valueOf(q, t); });
            var turned = lumitAngles(o._value, angles, near || angles);
            near = turned;
            // Held only where all three hold.
            var hold = trio.every(function (q) {
                var held = false;
                q._keys.forEach(function (k) { if (k.t <= t) held = k.hold; });
                return q._keys.length === 0 || held;
            });
            turned.forEach(function (v, n) {
                var k = new Key(t, v, frame);
                k.hold = hold;
                lists[n].push(k);
            });
        });
        trio.forEach(function (q, n) { write(layer, q._name, null, lists[n]); });
    }

    function writeLayer(layer) {
        if (layer._removed) return;
        writeRotations(layer);
        Object.keys(layer._props).forEach(function (name) {
            var prop = layer._props[name];
            if (!prop._changed || /^(rotation|rotation_x|rotation_y|orientation)$/.test(name)) return;
            write(layer, name, prop._value, prop._keys);
        });
        Object.keys(layer._props).forEach(function (name) { layer._props[name]._changed = false; });
    }

    function finish() {
        var now = dirty;
        dirty = [];
        now.forEach(writeLayer);
    }
    N.finish = finish;

    // ---- The project, app and the globals ----

    var project = guard({
        _at: undefined,
        get items() { return new ItemCollection(undefined); },
        get numItems() { return this.items.length; },
        item: function (n) { return this.items[n]; },
        get rootFolder() { return rootFolder; },
        get file() {
            var path = command('project.info').path;
            return path ? new File(path) : null;
        },
        importFile: function (options) {
            if (!(options instanceof ImportOptions)) throw new Error('importFile takes an ImportOptions.');
            if (options.importAs !== ImportAsType.FOOTAGE) unsupported('Importing as anything but footage');
            if (!options.file) throw new Error('ImportOptions has no file.');
            var file = options.file instanceof Entry ? options.file : new File(options.file);
            var made = command('footage.import', { paths: [file.fsName] }).items[0];
            var item = itemFor({ id: made.id, kind: 'footage' });
            if (!options.sequence && itemInfo(made.id).sequence) {
                note('A numbered still comes in as the whole sequence in Lumit.');
            }
            return item;
        },
        save: function (file) {
            command('project.save', file ? { path: file.fsName } : {});
            return true;
        },
    }, 'Project');

    function ImportOptions(file) {
        if (!(this instanceof ImportOptions)) return new ImportOptions(file);
        this.file = file || null;
        this.sequence = false;
        this.forceAlphabetical = false;
        this.importAs = ImportAsType.FOOTAGE;
        this._at = undefined;
        return guard(this, 'ImportOptions');
    }
    ImportOptions.prototype.canImportAs = function (as) { return as === ImportAsType.FOOTAGE; };

    globalThis.app = guard({
        _at: undefined,
        project: project,
        // A script works in the project that is open.
        newProject: function () {
            note('app.newProject() uses the open project.');
            return project;
        },
        // The whole run is already one undo step.
        beginUndoGroup: function () {},
        endUndoGroup: function () {},
    }, 'app');

    globalThis.$ = guard({
        _at: undefined,
        fileName: scriptPath || '',
        global: globalThis,
        writeln: function () { N.log(Array.prototype.map.call(arguments, String).join('')); },
        write: function () { N.log(Array.prototype.map.call(arguments, String).join('')); },
    }, '$');

    globalThis.alert = function (message) { N.alert(String(message)); };
    globalThis.writeLn = function (text) { N.log(String(text)); };
    globalThis.clearOutput = function () {};
    globalThis.File = File;
    globalThis.Folder = Folder;
    globalThis.ImportOptions = ImportOptions;
    globalThis.CompItem = CompItem;
    globalThis.FootageItem = FootageItem;
    globalThis.FolderItem = FolderItem;
    globalThis.AutoOrientType = AutoOrientType;
    globalThis.KeyframeInterpolationType = KeyframeInterpolationType;
    globalThis.ImportAsType = ImportAsType;
})
