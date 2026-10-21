// After Effects' ScriptUI, the controls an options dialog uses, drawn by
// Lumit. Run once, after ae.js and before the script.
//
// show() hands the interface the window as data and waits. Each press or
// change comes back with every control's value, the script's handlers run,
// and the window is shown again as they left it, until it closes. A palette
// waits the same way, as a script run cannot carry on behind a window.
(function (N) {
    var KINDS = ['group', 'panel', 'statictext', 'edittext', 'checkbox', 'radiobutton', 'dropdownlist', 'slider', 'button'];
    var made = 0;

    function unsupported(what) {
        throw new Error(what + ' is not supported in Lumit.');
    }

    // Fonts and colours are After Effects' own look, and Lumit draws its own.
    function Graphics() {}
    Graphics.prototype.newBrush = function () { return {}; };
    Graphics.prototype.newPen = function () { return {}; };

    function ListItem(list, text) {
        this.type = 'item';
        this.text = String(text);
        this._list = list;
    }
    Object.defineProperties(ListItem.prototype, {
        index: { get: function () { return this._list.items.indexOf(this); } },
        selected: {
            get: function () { return this._list.selection === this; },
            set: function (on) {
                if (on) this._list.selection = this;
                else if (this._list.selection === this) this._list.selection = null;
            },
        },
    });
    ListItem.prototype.toString = function () { return this.text; };

    function Control(window, type) {
        this.type = type;
        this.window = window;
        this.parent = null;
        this.children = [];
        this.text = '';
        this.enabled = true;
        this.visible = true;
        this.properties = {};
        this.graphics = new Graphics();
        this.orientation = type === 'group' ? 'row' : 'column';
        this.alignChildren = 'center';
        this.margins = type === 'group' ? 0 : 15;
        this.spacing = 10;
        this._id = 'c' + (++made);
        this._picked = null;
    }

    // add(type, bounds, text, properties), with the list for a dropdown and
    // value, least and most for a slider, as After Effects takes them.
    Control.prototype.add = function (type, bounds, a, b, c, d) {
        type = String(type);
        if (this.type === 'dropdownlist') {
            if (type !== 'item') unsupported('A dropdown list of ' + type);
            var item = new ListItem(this, bounds);
            this.items.push(item);
            return item;
        }
        if (KINDS.indexOf(type) < 0) unsupported('ScriptUI "' + type + '"');
        if (this.type !== 'group' && this.type !== 'panel' && !(this instanceof Window)) {
            throw new Error('Only a window, group or panel holds controls.');
        }
        var kid = new Control(this.window, type), props;
        kid.parent = this;
        if (type === 'dropdownlist') {
            kid.items = [];
            Array.prototype.forEach.call(a || [], function (t) { kid.add('item', t); });
            props = b;
        } else if (type === 'slider') {
            kid.value = a === undefined ? 0 : Number(a);
            kid.minvalue = b === undefined ? 0 : Number(b);
            kid.maxvalue = c === undefined ? 100 : Number(c);
            props = d;
        } else if (type === 'group') {
            props = a;
        } else {
            kid.text = a === undefined ? '' : String(a);
            props = b;
        }
        if (type === 'checkbox' || type === 'radiobutton') kid.value = false;
        if (props && typeof props === 'object') {
            kid.properties = props;
            if (props.name !== undefined) kid.name = String(props.name);
        }
        if (bounds instanceof Array && bounds.length === 4) {
            kid.preferredSize = [bounds[2] - bounds[0], bounds[3] - bounds[1]];
        }
        this.children.push(kid);
        return kid;
    };
    Control.prototype.remove = function (which) {
        var list = this.type === 'dropdownlist' ? this.items : this.children;
        var at = typeof which === 'number' ? which : list.indexOf(which);
        if (at >= 0 && at < list.length) {
            if (this.type === 'dropdownlist' && this.selection === list[at]) this.selection = null;
            list.splice(at, 1);
        }
    };
    Control.prototype.removeAll = function () {
        if (this.type === 'dropdownlist') {
            this.items = [];
            this._picked = null;
        } else {
            this.children = [];
        }
    };
    Control.prototype.find = function (text) {
        return (this.items || []).filter(function (i) { return i.text === String(text); })[0] || null;
    };
    // A dropdown's chosen item, set by item or by number.
    Object.defineProperty(Control.prototype, 'selection', {
        get: function () { return this._picked; },
        set: function (v) {
            if (typeof v === 'number') v = (this.items || [])[v] || null;
            this._picked = v instanceof ListItem && (this.items || []).indexOf(v) >= 0 ? v : null;
        },
    });
    Control.prototype.show = function () { this.visible = true; };
    Control.prototype.hide = function () { this.visible = false; };

    function Window(type, title, bounds, properties) {
        if (!(this instanceof Window)) return new Window(type, title, bounds, properties);
        type = String(type);
        if (!/^(dialog|palette|window)$/.test(type)) unsupported('A window of the kind "' + type + '"');
        Control.call(this, this, type);
        this.text = title === undefined ? '' : String(title);
        this.properties = properties || {};
        this.defaultElement = null;
        this.cancelElement = null;
        this.layout = { layout: function () {}, resize: function () {} };
        this._result = undefined;
        this._showing = false;
    }
    Window.prototype = Object.create(Control.prototype);
    Window.prototype.constructor = Window;
    Window.prototype.center = function () {};
    Window.prototype.hide = function () { this.close(0); };
    // What show() answers. A window closed with no number gives nought.
    Window.prototype.close = function (result) {
        this._result = result === undefined ? 0 : Number(result);
    };
    Window.prototype.show = function () {
        if (this._showing) return 0;
        this._showing = true;
        this._result = undefined;
        try {
            if (typeof this.onShow === 'function') this.onShow();
            while (this._result === undefined) {
                var byId = {};
                var tree = describe(this, byId);
                var heard = JSON.parse(N.dialog(JSON.stringify(tree)));
                // Closed by the person, or there is no window to show.
                if (heard === null) {
                    this._result = 2;
                } else {
                    answer(this, heard, byId);
                }
            }
        } finally {
            this._showing = false;
            N.dialog('');
        }
        if (typeof this.onClose === 'function') this.onClose();
        return this._result;
    };

    // OK is what Enter presses and Cancel what Escape does, named or
    // labelled so, unless the window says which.
    function role(w, c) {
        if (c.type !== 'button') return undefined;
        if (c === w.defaultElement) return 'ok';
        if (c === w.cancelElement) return 'cancel';
        if (w.defaultElement || w.cancelElement) return undefined;
        var said = String(c.name || c.text).toLowerCase();
        return said === 'ok' || said === 'cancel' ? said : undefined;
    }

    // A side as [across, down], from one word or two.
    function sides(v) {
        if (v instanceof Array) return [String(v[0]), String(v[1] === undefined ? v[0] : v[1])];
        return [String(v), String(v)];
    }

    function describe(c, byId) {
        byId[c._id] = c;
        var d = {
            id: c._id,
            type: c.type,
            text: String(c.text === undefined || c.text === null ? '' : c.text),
            enabled: c.enabled !== false,
            visible: c.visible !== false,
            handles: !!(c.onClick || c.onChange || c.onChanging),
        };
        if (c.alignment) d.alignment = sides(c.alignment);
        var size = c.preferredSize || c.size;
        if (size && Number(size[0]) > 0) d.width = Number(size[0]);
        else if (c.characters) d.width = Number(c.characters) * 7;
        if (c.helpTip) d.help = String(c.helpTip);
        if (c.type === 'group' || c.type === 'panel' || c instanceof Window) {
            var m = c.margins instanceof Array ? c.margins : [c.margins, c.margins, c.margins, c.margins];
            d.orientation = String(c.orientation);
            d.alignChildren = sides(c.alignChildren);
            d.margins = Array.prototype.map.call(m, function (x) { return Number(x) || 0; });
            d.spacing = Number(c.spacing) || 0;
            d.children = c.children.map(function (k) { return describe(k, byId); });
        }
        if (c.type === 'checkbox' || c.type === 'radiobutton') d.value = !!c.value;
        if (c.type === 'slider') {
            d.value = Number(c.value) || 0;
            d.min = Number(c.minvalue) || 0;
            d.max = Number(c.maxvalue) || 0;
        }
        if (c.type === 'dropdownlist') {
            d.items = c.items.map(function (i) { return i.text; });
            d.selection = c.selection ? c.selection.index : null;
        }
        if (c.type === 'edittext' || c.type === 'statictext') d.multiline = !!c.properties.multiline;
        if (c.type === 'edittext') d.readonly = !!c.properties.readonly;
        var r = role(c.window, c);
        if (r) d.role = r;
        return d;
    }

    // What the person did: every value as it now stands, and the control
    // pressed or changed. Changes run onChanging and onChange, then a press
    // runs onClick, and OK or Cancel with none of its own closes the window.
    function answer(w, heard, byId) {
        var values = heard.values || {}, changed = [];
        Object.keys(values).forEach(function (id) {
            var c = byId[id], v = values[id];
            if (!c) return;
            if (c.type === 'edittext' && String(v) !== String(c.text)) {
                c.text = String(v);
                changed.push(c);
            } else if (c.type === 'checkbox' || c.type === 'radiobutton') {
                c.value = !!v;
            } else if (c.type === 'slider' && Number(v) !== Number(c.value)) {
                c.value = Number(v);
                changed.push(c);
            } else if (c.type === 'dropdownlist' && (c.selection ? c.selection.index : null) !== v) {
                c.selection = v === null ? null : Number(v);
                changed.push(c);
            }
        });
        changed.forEach(function (c) {
            if (typeof c.onChanging === 'function') c.onChanging();
            if (typeof c.onChange === 'function') c.onChange();
        });
        var pressed = byId[heard.id];
        if (!pressed || !/^(button|checkbox|radiobutton)$/.test(pressed.type)) return;
        if (typeof pressed.onClick === 'function') {
            pressed.onClick();
            return;
        }
        var r = role(w, pressed);
        if (r) w.close(r === 'ok' ? 1 : 2);
    }

    // A dockable panel is never handed to a script here, so a script that
    // asks `instanceof Panel` makes its own window.
    function Panel() { unsupported('A dockable panel'); }

    globalThis.Window = Window;
    globalThis.Panel = Panel;
    globalThis.ScriptUI = {
        environment: { keyboardState: {} },
        FontStyle: { REGULAR: 0, BOLD: 1, ITALIC: 2, BOLD_ITALIC: 3 },
        BrushType: { SOLID_COLOR: 0, THEME_COLOR: 1 },
        PenType: { SOLID_COLOR: 0, THEME_COLOR: 1 },
        newFont: function () { return {}; },
    };
})
