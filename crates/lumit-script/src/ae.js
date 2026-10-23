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

    // One side of a key's ease: a speed in units a second, and how far the
    // handle reaches as a per cent of the span.
    function KeyframeEase(speed, influence) {
        if (!(this instanceof KeyframeEase)) return new KeyframeEase(speed, influence);
        this.speed = Number(speed);
        this.influence = Number(influence);
        this._at = undefined;
        return guard(this, 'KeyframeEase');
    }

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
        // What has been written since the file was opened to write, and
        // whether it goes after what is there.
        this._out = null;
        this._append = false;
        this._at = undefined;
        this.encoding = 'UTF-8';
        this.lineFeed = windows ? 'Windows' : 'Unix';
        return guard(this, 'File');
    }
    File.prototype = Object.create(Entry.prototype);
    File.prototype.constructor = File;
    File.prototype.open = function (mode) {
        var m = String(mode || 'r').charAt(0);
        if (m === 'w' || m === 'a') {
            if (!/^utf-?8$/i.test(this.encoding)) {
                throw new Error('A script writes text as UTF-8 only, so ' + this.encoding + ' is not supported in Lumit.');
            }
            // Refused here rather than at close, so the script hears it on
            // the line that asked.
            N.mayWrite(this._path);
            this._out = '';
            this._append = m === 'a';
            if (writing.indexOf(this) < 0) writing.push(this);
            return true;
        }
        if (m !== 'r') unsupported('File.open("' + mode + '")');
        this._text = JSON.parse(N.readText(this._path));
        this._read = 0;
        return true;
    };
    File.prototype.write = function () {
        if (this._out === null) throw new Error(this.name + ' is not open for writing.');
        this._out += Array.prototype.map.call(arguments, String).join('');
        return true;
    };
    File.prototype.writeln = function () {
        this.write.apply(this, arguments);
        this._out += { Windows: '\r\n', Macintosh: '\r' }[this.lineFeed] || '\n';
        return true;
    };
    File.prototype.remove = function () {
        N.removeFile(this._path);
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
    File.prototype.close = function () {
        this._text = null;
        flush(this);
        return true;
    };

    // Files open to write. What was written goes to disk when each is
    // closed, or when the script ends.
    var writing = [];
    function flush(file) {
        if (file._out === null) return;
        var text = file._out;
        file._out = null;
        writing.splice(writing.indexOf(file), 1);
        if (file._append) N.appendText(file._path, text);
        else N.writeText(file._path, text);
    }
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
    Folder.prototype.create = function () {
        N.makeFolder(this._path);
        return true;
    };
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
        markerProperty: { get: function () { return new MarkerProperty(this, null); } },
    });
    CompItem.prototype.layer = function (which) {
        return typeof which === 'string' ? this.layers.byName(which) : this.layers.byIndex(which);
    };
    CompItem.prototype.openInViewer = function () {
        command('comp.open', { comp: this._id });
        return null;
    };
    CompItem.prototype.saveFrameToPng = function (time, file) {
        var to = file instanceof Entry ? file : new File(file);
        // The keys made so far go in first, so the frame shows them.
        finish();
        command('comp.frame', { comp: this._id, time: this._frame(time), out: to.fsName });
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
        var layer;
        if (item instanceof CompItem) {
            layer = this._add('precomp', undefined, item._id);
        } else if (item instanceof FootageItem) {
            layer = this._add('footage', undefined, item._id);
            var info = layer._info();
            item._duration = seconds(info.out) - seconds(info.in);
        } else {
            unsupported('layers.add of a ' + (item && item.typeName));
        }
        if (duration !== undefined) layer.outPoint = layer.inPoint + Number(duration);
        return layer;
    };
    // Layers by After Effects' numbers, from 1. Every attribute moves with
    // them unless told otherwise, which only a single layer may be.
    LayerCollection.prototype.precompose = function (indices, name, moveAllAttributes) {
        var list = this._list(), comp = this._comp;
        var ids = Array.prototype.map.call(indices, function (n) {
            var l = list[n - 1];
            if (!l) throw new Error('There is no layer ' + n + ' in ' + comp.name + '.');
            return l.id;
        });
        // The keys made so far go in while the layers are still here.
        finish();
        var made = command('comp.precompose', {
            comp: comp._id,
            layers: ids,
            name: name === undefined ? '' : String(name),
            leave_attributes: moveAllAttributes === false,
        });
        var inner = itemFor({ id: made.comp.id, kind: 'comp' });
        // Moved whole, a layer keeps its id in the new comp.
        if (moveAllAttributes !== false) {
            ids.forEach(function (id) { if (layers[id]) layers[id]._comp = inner; });
        }
        return inner;
    };
    LayerCollection.prototype.addSolid = function (color, name, width, height, pixelAspect, duration) {
        if (pixelAspect !== undefined && Math.abs(Number(pixelAspect) - 1) > 1e-3) {
            note('Lumit solids have square pixels, so a pixel aspect of ' + pixelAspect + ' was left out.');
        }
        var made = command('layer.add', {
            comp: this._comp._id,
            kind: 'solid',
            name: String(name),
            colour: [0, 1, 2].map(function (n) { return Math.min(1, Math.max(0, Number(color[n]))); }),
            width: Math.round(width),
            height: Math.round(height),
        });
        var layer = layerFor(this._comp, made.id);
        if (duration !== undefined) layer.outPoint = Number(duration);
        return layer;
    };

    function Layer(comp, id) {
        this._comp = comp;
        this._id = id;
        this._at = undefined;
        this._props = {};
        // Effect controls set in this run, written with the keys.
        this._fx = [];
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
        Effects: { get: function () { return new EffectGroup(this); } },
        effect: {
            get: function () {
                var group = new EffectGroup(this);
                return function (which) { return group.property(which); };
            },
        },
        marker: { get: function () { return new MarkerProperty(this._comp, this); } },
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
        'ADBE Effect Parade': 'effects', 'Effects': 'effects',
        'ADBE Marker': 'marker', 'Marker': 'marker',
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
        if (group === 'effects') return new EffectGroup(this);
        if (group === 'marker') return new MarkerProperty(this._comp, this);
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

    var T = KeyframeInterpolationType;

    function Key(t, v, frame) {
        this.t = t;
        this.v = v;
        this.f = frame;
        this.inType = T.LINEAR;
        this.outType = T.LINEAR;
        // Lists of KeyframeEase, once the script or the document gives some.
        this.inEase = null;
        this.outEase = null;
        // Temporal auto-Bezier, and continuous, which is kept only to be read.
        this.auto = false;
        this.cont = false;
    }
    // Held means held leaving the key, and Lumit then holds both sides.
    Object.defineProperty(Key.prototype, 'hold', {
        get: function () { return this.outType === T.HOLD; },
        set: function (on) {
            if (on) this.inType = this.outType = T.HOLD;
            else if (this.outType === T.HOLD) this.inType = this.outType = T.LINEAR;
        },
    });
    // How `from` meets the curve, given to this key.
    Key.prototype._sides = function (from) {
        this.inType = from.inType;
        this.outType = from.outType;
        this.inEase = from.inEase;
        this.outEase = from.outEase;
        this.auto = from.auto;
        this.cont = from.cont;
    };

    // A key's sides as the document holds them: the first axis with a key at
    // `t` gives the interpolation, and each axis its own ease.
    function sidesFrom(k, channels, t) {
        var at = channels.map(function (c) {
            return c instanceof Array ? c.filter(function (x) { return seconds(x.time) === t; })[0] : undefined;
        });
        var first = at.filter(Boolean)[0];
        if (!first) return;
        function type(side) {
            if (side === 'hold') return T.HOLD;
            return side === 'linear' || side === undefined ? T.LINEAR : T.BEZIER;
        }
        function eases(side) {
            var list = at.map(function (x) {
                var s = x && x[side];
                return s && typeof s === 'object' ? new KeyframeEase(s.speed, s.influence) : null;
            });
            return list.some(Boolean) ? list.map(function (e) { return e || new KeyframeEase(0, 100 / 3); }) : null;
        }
        k.inType = type(first['in']);
        k.outType = type(first.out);
        k.inEase = eases('in');
        k.outEase = eases('out');
        k.auto = /^(auto|clamped)$/.test(first['in']) || /^(auto|clamped)$/.test(first.out);
    }

    function Property(layer, name) {
        this._layer = layer;
        this._name = name;
        this._keys = null;
        this._value = null;
        // An expression, and whether it drives the property.
        this._expr = '';
        this._exprOn = false;
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
        fill(this, AXES[name] ? held : [held], function (values) {
            return aeValue(name, AXES[name] ? values : values[0]);
        });
    };

    // The value, keys and expression After Effects sees in `channels`, one
    // for each of Lumit's axes. `make` turns their numbers into its value.
    function fill(prop, channels, make) {
        channels.forEach(function (c) {
            if (c && c.expression !== undefined) {
                prop._expr = c.expression;
                prop._exprOn = true;
            }
        });
        prop._value = make(channels.map(function (c) { return typeof c === 'number' ? c : 0; }));
        var times = channelTimes(channels);
        for (var i = 0; i < times.length; i++) {
            var t = times[i];
            var k = new Key(t, make(channels.map(function (c) { return channelAt(c, t); })), prop._layer._comp._frame(t));
            sidesFrom(k, channels, t);
            k.hold = channels.every(function (c) { return heldAt(c, t); });
            prop._keys.push(k);
        }
    }

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
                key._sides(keys[i]);
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
        // Setting an expression switches it on, and an empty one takes it off.
        expression: {
            get: function () { this._load(); return this._expr; },
            set: function (text) {
                this._load();
                this._expr = String(text);
                this._exprOn = this._expr !== '';
                this._dirty();
            },
        },
        expressionEnabled: {
            get: function () { this._load(); return this._exprOn; },
            set: function (on) {
                this._load();
                this._exprOn = !!on && this._expr !== '';
                this._dirty();
            },
        },
        canSetExpression: { value: true },
        expressionError: { get: function () { return ''; } },
        // After Effects moves a point property along a path.
        isSpatial: { get: function () { return SPATIAL.indexOf(this._name) >= 0; } },
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
    function interpolation(type) {
        if (type !== T.LINEAR && type !== T.BEZIER && type !== T.HOLD) {
            throw new Error(type + ' is not a KeyframeInterpolationType.');
        }
        return type;
    }
    Property.prototype.isInterpolationTypeValid = function (type) {
        return type === T.LINEAR || type === T.BEZIER || type === T.HOLD;
    };
    // A key turned Bezier with no ease of its own follows its neighbours, as
    // an auto-Bezier key does.
    Property.prototype.setInterpolationTypeAtKey = function (k, inType, outType) {
        var key = this._key(k);
        key.inType = interpolation(inType);
        key.outType = interpolation(outType === undefined ? inType : outType);
        if (!key.inEase && !key.outEase) key.auto = true;
        this._dirty();
    };
    Property.prototype.keyInInterpolationType = function (k) { return this._key(k).inType; };
    Property.prototype.keyOutInterpolationType = function (k) { return this._key(k).outType; };

    // How many eases a key's side has: one for a point that moves along a
    // path, otherwise one for each value.
    var SPATIAL = ['position', 'anchor', 'point_of_interest'];
    Property.prototype._eases = function () {
        if (SPATIAL.indexOf(this._name) >= 0) return 1;
        var v = this._key(1).v;
        return v instanceof Array ? v.length : 1;
    };
    function eases(prop, list) {
        if (!(list instanceof Array) || !list.length) throw new Error('An ease is a list of KeyframeEase.');
        var n = prop._eases();
        if (list.length !== n && list.length !== 1) throw new Error(prop.name + ' takes ' + n + ' KeyframeEase.');
        return Array.prototype.map.call(list, function (e) {
            var influence = Number(e.influence);
            if (!(influence >= 0.1 && influence <= 100)) throw new Error('An influence is 0.1 to 100.');
            return new KeyframeEase(Number(e.speed), influence);
        });
    }
    // An ease makes a side Bezier, as in After Effects.
    Property.prototype.setTemporalEaseAtKey = function (k, inEase, outEase) {
        var key = this._key(k);
        key.inEase = eases(this, inEase);
        key.outEase = eases(this, outEase === undefined ? inEase : outEase);
        key.inType = key.outType = T.BEZIER;
        key.auto = false;
        this._dirty();
    };
    // A side with no ease of its own, as After Effects starts one: no speed,
    // and a sixth of the span.
    Property.prototype._ease = function (k, out) {
        var key = this._key(k), list = out ? key.outEase : key.inEase;
        var n = this._eases();
        return Array.apply(null, Array(n)).map(function (_, i) {
            var e = list && (list[i] || list[0]);
            return e ? new KeyframeEase(e.speed, e.influence) : new KeyframeEase(0, 100 / 6);
        });
    };
    Property.prototype.keyInTemporalEase = function (k) { return this._ease(k, false); };
    Property.prototype.keyOutTemporalEase = function (k) { return this._ease(k, true); };
    Property.prototype.setTemporalAutoBezierAtKey = function (k, on) {
        var key = this._key(k);
        key.auto = !!on;
        if (on) key.inType = key.outType = T.BEZIER;
        this._dirty();
    };
    Property.prototype.keyTemporalAutoBezier = function (k) { return this._key(k).auto; };
    // Lumit's eases are each side's own, so continuous is only kept to be read.
    Property.prototype.setTemporalContinuousAtKey = function (k, on) { this._key(k).cont = !!on; };
    Property.prototype.keyTemporalContinuous = function (k) { return this._key(k).cont; };
    Property.prototype.setRovingAtKey = function (k, on) {
        this._key(k);
        if (on) note('Roving keys stay at their times in Lumit.');
    };
    Property.prototype.keyRoving = function (k) { this._key(k); return false; };

    // Lumit moves a point between keys in a straight line, so these calls
    // can only ask for that or leave it as it is.
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
    Property.prototype._tangent = function (k) {
        var v = this._key(k).v;
        return v instanceof Array ? v.map(function () { return 0; }) : [0];
    };
    Property.prototype.keyInSpatialTangent = Property.prototype._tangent;
    Property.prototype.keyOutSpatialTangent = Property.prototype._tangent;
    Property.prototype.keySpatialAutoBezier = function (k) { this._key(k); return false; };
    Property.prototype.keySpatialContinuous = function (k) { this._key(k); return false; };

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

    // One side of a key as Lumit's commands take it.
    function sideOf(k, out) {
        var type = out ? k.outType : k.inType, ease = out ? k.outEase : k.inEase;
        if (type === T.HOLD) return 'hold';
        if (type === T.LINEAR) return 'linear';
        if (k.auto || !ease) return 'auto';
        return ease.map(function (e) { return { speed: e.speed, influence: e.influence }; });
    }

    // `keys` as Lumit's commands take them, each value through `value`.
    function keyList(layer, keys, value) {
        var out = [];
        keys.forEach(function (k) {
            var key = { time: layer._comp._frame(k.t), value: value(k.v), 'in': sideOf(k, false), out: sideOf(k, true) };
            // Two keys on one frame: the later wins, as in After Effects.
            if (out.length && out[out.length - 1].time === key.time) out[out.length - 1] = key;
            else out.push(key);
        });
        return out;
    }

    function write(layer, name, value, keys) {
        var args = { comp: layer._comp._id, layer: layer._id, property: name };
        if (keys && keys.length) {
            args.keys = keyList(layer, keys, function (v) { return lumitValue(name, v); });
        } else {
            args.value = lumitValue(name, value);
        }
        command('layer.keys', args);
    }

    // A property's expression while it is on, as Lumit holds one or the other.
    function writeProp(layer, prop) {
        if (!prop._exprOn) {
            write(layer, prop._name, prop._value, prop._keys);
            return;
        }
        if (prop._keys.length) note('Keys under an expression are not kept, as a Lumit property holds one or the other.');
        command('layer.keys', { comp: layer._comp._id, layer: layer._id, property: prop._name, expression: prop._expr });
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
                sidesFrom(k, channels, t);
                k.hold = hold;
                lists[n].push(k);
            });
        });
        TURNS.forEach(function (name, n) {
            var p = layer._property(name);
            if (p !== caller && p._keys !== null) return;
            p._keys = lists[n];
            p._value = still[n];
            var c = channels[n];
            if (c && c.expression !== undefined) {
                p._expr = c.expression;
                p._exprOn = true;
            }
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
        if (o._exprOn) unsupported('An expression on Orientation');
        var info = layer._info();
        var solid = info.kind === 'camera' || info.three_d;
        var trio = [p.rotation_x, p.rotation_y, p.rotation];
        // An expression is written as it is, so its rotation is not turned.
        var expressed = trio.some(function (q) { return q._exprOn; });
        if (!solid || expressed || (!used(o) && !(used(p.rotation_x) && used(p.rotation_y)))) {
            if (used(o)) unsupported(solid ? 'Orientation beside an expression on a rotation' : 'Orientation on a 2D layer');
            trio.forEach(function (q) {
                if (q._changed) writeProp(layer, q);
            });
            return;
        }
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
            // The turn is split again, so a key's ease, which was for the
            // old angles, is not carried. A Bezier key follows its neighbours.
            var from = null;
            trio.forEach(function (q) {
                q._keys.forEach(function (k) { if (!from && k.f === frame) from = k; });
            });
            turned.forEach(function (v, n) {
                var k = new Key(t, v, frame);
                if (from) {
                    k.inType = from.inType;
                    k.outType = from.outType;
                    k.auto = true;
                }
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
            writeProp(layer, prop);
        });
        Object.keys(layer._props).forEach(function (name) { layer._props[name]._changed = false; });
        layer._fx.forEach(function (prop) {
            if (prop._changed) writeControl(prop);
            prop._changed = false;
        });
    }

    function finish() {
        var now = dirty;
        dirty = [];
        now.forEach(writeLayer);
    }
    // At the end of the script the keys are written, and so are files it did
    // not close.
    N.finish = function () {
        finish();
        writing.slice().forEach(flush);
    };

    // ---- Effects ----

    var effects = {};
    function effectFor(layer, info) {
        var made = effects[info.id] || (effects[info.id] = new Effect(layer, info.id));
        // The After Effects effect it stands for names its controls.
        if (!made._ae) made._ae = info.after_effects || null;
        return made;
    }

    function EffectGroup(layer) {
        this._layer = layer;
        this._at = function (n) { return this.property(n); };
        return guard(this, 'Effects');
    }
    EffectGroup.prototype._list = function () {
        return command('effect.list', { comp: this._layer._comp._id, layer: this._layer._id }).effects;
    };
    Object.defineProperties(EffectGroup.prototype, {
        name: { value: 'Effects' },
        matchName: { value: 'ADBE Effect Parade' },
        numProperties: { get: function () { return this._list().length; } },
    });
    EffectGroup.prototype.addProperty = function (name) {
        var made = command('effect.add', { comp: this._layer._comp._id, layer: this._layer._id, effect: String(name) });
        return effectFor(this._layer, made);
    };
    EffectGroup.prototype.property = function (which) {
        var list = this._list(), found;
        if (typeof which === 'number') {
            found = list[which - 1];
        } else {
            found = list.filter(function (e) {
                return e.name === which || e.effect === which || e.after_effects === which;
            })[0];
        }
        return found ? effectFor(this._layer, found) : null;
    };

    function Effect(layer, id) {
        this._layer = layer;
        this._id = id;
        this._ae = null;
        this._props = {};
        this._at = function (n) { return this.property(n); };
        return guard(this, 'Effect');
    }
    Effect.prototype._info = function () {
        var id = this._id;
        var found = new EffectGroup(this._layer)._list().filter(function (e) { return e.id === id; })[0];
        if (!found) throw new Error('That effect is no longer on the layer.');
        return found;
    };
    Effect.prototype._set = function (args) {
        args.comp = this._layer._comp._id;
        args.layer = this._layer._id;
        args.effect = this._id;
        return command('effect.set', args);
    };
    var switchedOn = {
        get: function () { return this._info().enabled; },
        set: function (v) { this._set({ enabled: !!v }); },
    };
    Object.defineProperties(Effect.prototype, {
        name: {
            get: function () { return this._info().name; },
            set: function (v) { this._set({ name: String(v) }); },
        },
        matchName: { get: function () { return this._ae || this._info().effect; } },
        enabled: switchedOn,
        active: switchedOn,
        numProperties: { get: function () { return this._info().params.length; } },
        propertyIndex: { get: function () { return this._info().index + 1; } },
    });
    // After Effects counts an effect's controls from one and names them after
    // the effect, which is how one is found by number.
    Effect.prototype.property = function (which) {
        var key = which;
        if (typeof which === 'number') {
            if (this._ae) {
                key = this._ae + '-' + ('000' + which).slice(-4);
            } else {
                var own = this._info().params[which - 1];
                if (!own) throw new Error(this.name + ' has no control ' + which + '.');
                key = own.id;
            }
        }
        key = String(key);
        return this._props[key] || (this._props[key] = new EffectProperty(this, key));
    };

    // One of an effect's controls. Named the way After Effects names it, it
    // takes After Effects' values, which Lumit converts as importing does.
    function EffectProperty(effect, key) {
        Property.call(this, effect._layer, key);
        this._effect = effect;
        return guard(this, 'Property');
    }
    EffectProperty.prototype = Object.create(Property.prototype);
    EffectProperty.prototype.constructor = EffectProperty;
    // Lumit's own control by this name, if it is one.
    EffectProperty.prototype._own = function () {
        var name = this._name, lower = name.toLowerCase();
        return this._effect._info().params.filter(function (p) {
            return p.id === name || p.label.toLowerCase() === lower;
        })[0];
    };
    // An After Effects control starts empty, as Lumit cannot read its value
    // back in After Effects' units.
    EffectProperty.prototype._load = function () {
        if (this._keys !== null) return;
        this._keys = [];
        var own = this._own();
        if (!own) return;
        if (own.kind === 'float') fill(this, [own.value], function (values) { return values[0]; });
        else if (own.kind === 'point' || own.kind === 'colour') fill(this, own.value, function (values) { return values; });
        else this._value = own.value;
    };
    EffectProperty.prototype._checked = function (v) {
        if (v instanceof Array) return Array.prototype.map.call(v, Number);
        return typeof v === 'boolean' ? (v ? 1 : 0) : Number(v);
    };
    EffectProperty.prototype._dirty = function () {
        Property.prototype._dirty.call(this);
        if (this._layer._fx.indexOf(this) < 0) this._layer._fx.push(this);
    };
    Object.defineProperties(EffectProperty.prototype, {
        name: {
            get: function () {
                var own = this._own();
                return own ? own.label : this._name;
            },
        },
        matchName: { get: function () { return this._name; } },
        isSpatial: { value: false },
    });

    function writeControl(prop) {
        var args = { param: prop._name };
        if (prop._exprOn) args.expression = prop._expr;
        else if (prop._keys.length) args.keys = keyList(prop._layer, prop._keys, function (v) { return v; });
        else if (prop._value !== null) args.value = prop._value;
        else return;
        prop._effect._set(args);
    }

    // ---- Markers ----

    function MarkerValue(comment, chapter, url, frameTarget, cuePointName, params) {
        if (!(this instanceof MarkerValue)) return new MarkerValue(comment, chapter, url, frameTarget, cuePointName, params);
        function text(v) { return v === undefined ? '' : String(v); }
        this.comment = text(comment);
        this.chapter = text(chapter);
        this.url = text(url);
        this.frameTarget = text(frameTarget);
        this.cuePointName = text(cuePointName);
        this.eventCuePoint = false;
        this.duration = 0;
        this.label = 0;
        this.protectedRegion = false;
        this._params = params || {};
        this._at = undefined;
        return guard(this, 'MarkerValue');
    }
    MarkerValue.prototype.getParameters = function () { return this._params; };
    MarkerValue.prototype.setParameters = function (params) { this._params = params || {}; };

    // A comp's markers, or a layer's when there is one, written as they are
    // set.
    function MarkerProperty(comp, layer) {
        this._comp = comp;
        this._layer = layer;
        this._at = undefined;
        return guard(this, 'Marker');
    }
    MarkerProperty.prototype._list = function () {
        return (this._layer ? this._layer._info() : this._comp._get()).markers;
    };
    MarkerProperty.prototype._write = function (list) {
        var comp = this._comp;
        function at(m) { return typeof m.time === 'number' ? m.time * comp.frameDuration : seconds(m.time); }
        list.sort(function (a, b) { return at(a) - at(b); });
        var args = { comp: comp._id, markers: list };
        if (this._layer) args.layer = this._layer._id;
        command('markers.set', args);
    };
    MarkerProperty.prototype._marker = function (k) {
        var m = this._list()[k - 1];
        if (!m) throw new Error('There is no marker ' + k + '.');
        return m;
    };
    Object.defineProperties(MarkerProperty.prototype, {
        name: { value: 'Marker' },
        matchName: { value: 'ADBE Marker' },
        numKeys: { get: function () { return this._list().length; } },
        isTimeVarying: { get: function () { return this.numKeys > 0; } },
    });
    MarkerProperty.prototype.keyTime = function (k) { return seconds(this._marker(k).time); };
    // Lumit keeps a marker's comment as its label, and a chapter marker as
    // one, without text of its own.
    MarkerProperty.prototype.keyValue = function (k) {
        var m = this._marker(k);
        var value = new MarkerValue(m.label, m.chapter ? m.label : '');
        if (m.duration) value.duration = m.duration * this._comp.frameDuration;
        value.label = m.colour || 0;
        return value;
    };
    MarkerProperty.prototype.nearestKeyIndex = function (t) {
        var list = this._list(), best = 0;
        for (var i = 1; i < list.length; i++) {
            if (Math.abs(seconds(list[i].time) - t) < Math.abs(seconds(list[best].time) - t)) best = i;
        }
        return list.length ? best + 1 : 0;
    };
    MarkerProperty.prototype.setValueAtTime = function (t, value) {
        if (!(value instanceof MarkerValue)) throw new Error('A marker is set with a MarkerValue.');
        if (value.url || value.frameTarget || value.cuePointName || value.eventCuePoint
            || (value.chapter && value.chapter !== value.comment)) {
            note('A marker keeps its comment and whether it starts a chapter. Lumit markers have no chapter text, URL or cue point.');
        }
        var comp = this._comp, frame = comp._frame(t);
        var list = this._list().filter(function (m) { return comp._frame(seconds(m.time)) !== frame; });
        var made = { time: frame, label: value.comment, chapter: value.chapter !== '' };
        if (value.label > 0) made.colour = Math.round(value.label);
        var span = comp._frame(value.duration);
        if (span > 0) made.duration = span;
        list.push(made);
        this._write(list);
    };
    MarkerProperty.prototype.setValuesAtTimes = function (times, values) {
        if (times.length !== values.length) throw new Error('setValuesAtTimes needs a value for each time.');
        for (var i = 0; i < times.length; i++) this.setValueAtTime(times[i], values[i]);
    };
    MarkerProperty.prototype.addKey = function (t) {
        this.setValueAtTime(t, new MarkerValue(''));
        return this.nearestKeyIndex(t);
    };
    MarkerProperty.prototype.removeKey = function (k) {
        this._marker(k);
        var list = this._list();
        list.splice(k - 1, 1);
        this._write(list);
    };

    // ---- The project, app and the globals ----

    var project = guard({
        _at: undefined,
        get items() { return new ItemCollection(undefined); },
        get numItems() { return this.items.length; },
        item: function (n) { return this.items[n]; },
        get rootFolder() { return rootFolder; },
        // The comp open in the Viewer.
        get activeItem() {
            var comp = command('comp.active').comp;
            return comp ? itemFor({ id: comp.id, kind: 'comp' }) : null;
        },
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
            // The keys made so far go in first, so the file has them.
            finish();
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
        // The whole run is already one undo step, which takes the first
        // name given here.
        beginUndoGroup: function (name) {
            if (name !== undefined) N.nameStep(String(name));
        },
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
    globalThis.KeyframeEase = KeyframeEase;
    globalThis.MarkerValue = MarkerValue;
})
