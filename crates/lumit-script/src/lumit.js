// The `lumit` object every script sees, built over the few functions the
// runner hands in as `N`. Run once, before the script.
(function (N, commands, args) {
    function command(name, a) {
        return JSON.parse(N.call(String(name), JSON.stringify(a === undefined ? null : a)));
    }

    function line(parts) {
        return Array.prototype.map.call(parts, String).join(' ');
    }

    var lumit = {
        command: command,
        args: args,
        chooseFile: function (title, extensions) {
            return JSON.parse(N.chooseFile(String(title || ''), JSON.stringify(extensions || [])));
        },
        readText: function (path) {
            return JSON.parse(N.readText(String(path)));
        },
        readJson: function (path) {
            var text = lumit.readText(path);
            try {
                return JSON.parse(text);
            } catch (e) {
                throw new Error(path + ' is not JSON: ' + e.message);
            }
        },
        // The frame of a comp nearest another program's time in seconds.
        frameAt: function (seconds, comp) {
            var rate = command('comp.get', { comp: comp }).frame_rate;
            return N.frameOn(Number(seconds), rate.num, rate.den);
        },
    };

    // Every command as a method of its own, `layer.add` as lumit.layer.add.
    // Undo and redo are left out, as a run is one step.
    commands.forEach(function (name) {
        if (name === 'undo' || name === 'redo') return;
        var parts = name.split('.'), at = lumit;
        for (var i = 0; i < parts.length - 1; i++) at = at[parts[i]] || (at[parts[i]] = {});
        at[parts[parts.length - 1]] = function (a) { return command(name, a); };
    });

    globalThis.lumit = lumit;
    globalThis.print = function () { N.log(line(arguments)); };
    globalThis.console = {
        log: globalThis.print,
        info: globalThis.print,
        warn: globalThis.print,
        error: globalThis.print,
    };
})
