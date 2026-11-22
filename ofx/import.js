// One temporary composition, built by Lumit's existing scripting commands.
var s = lumit.readJson(lumit.args.selection);
function time(v) { return {num:Math.round(v * 705600000),den:705600000}; }
var c = lumit.comp.new({name:s.name, width:s.width, height:s.height,
    frame_rate:s.rate, duration:time(s.duration), background:[0,0,0,0]});
// Times below are exact seconds, including subframe reference-audio edits.
var media = {}, keep = {};
s.clips.forEach(function (clip) {
    var layer;
    var scale = 100;
    if (clip.path && !clip.unsupported) {
        try {
            var item = media[clip.path];
            if (!item) {
                var id = lumit.footage.import({paths:[clip.path]}).items[0].id;
                item = lumit.item.list().items.filter(function(i) { return i.id===id; })[0];
                if (!item.media) throw new Error('Media is unavailable');
                media[clip.path] = item;
            }
            layer = lumit.layer.add({comp:c.id,kind:'footage',item:item.id,name:clip.name});
            if (clip.kind==='video' && item.media.width && item.media.height)
                scale = 100*Math.min(s.width/item.media.width,s.height/item.media.height);
        } catch (e) { clip.unsupported = 'Media could not be imported'; }
    }
    if (!layer) layer = lumit.layer.add({comp:c.id,kind:'null',name:'[Placeholder] '+clip.name});
    // A source file can make extra sound rows; keep only this selected item.
    lumit.comp.get({comp:c.id}).layers.forEach(function (l) {
        if (l.id !== layer.id && !keep[l.id]) lumit.layer.delete({comp:c.id,layer:l.id});
    });
    keep[layer.id] = true;
    lumit.layer.set({comp:c.id,layer:layer.id,name:(clip.unsupported?'[Placeholder] ':'')+clip.name,
        in:time(clip.start),out:time(clip.end),start_offset:time(clip.start-clip.source),
        switches:{visible:clip.kind==='video' && clip.enabled,audible:clip.kind==='audio' && clip.enabled}});
    lumit.layer.write({comp:c.id,layer:layer.id,fields:{audio_only:clip.kind==='audio',
        transform:{scale_x:{animation:{Static:scale}},scale_y:{animation:{Static:scale}}},
        volume_db:{animation:{Static:clip.volume||0}}}});
});
print(JSON.stringify({composition:c.id,name:c.name}));
