# Run inside the user's Blender: bring one file in and write it out as GLB.
# Called as `blender -b [in.blend] ... --python-expr <this> -- <in> <out>`.
import os
import sys

import bpy

source, out = sys.argv[sys.argv.index("--") + 1:][:2]

ext = os.path.splitext(source)[1].lower()
if ext != ".blend":
    importers = {
        ".usd": "wm.usd_import",
        ".usdz": "wm.usd_import",
        ".usdc": "wm.usd_import",
        ".usda": "wm.usd_import",
        ".abc": "wm.alembic_import",
        ".ply": "wm.ply_import",
        ".stl": "wm.stl_import",
        ".dae": "wm.collada_import",
        ".fbx": "import_scene.fbx",
    }
    group, name = importers[ext].split(".")
    try:
        importer = getattr(getattr(bpy.ops, group), name)
        importer.get_rna_type()
    except (AttributeError, KeyError):
        # This Blender has no importer for it (Collada went in 5.0).
        sys.exit(3)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    importer(filepath=source)

# glTF has nowhere for a lens that moves, so each camera that has one writes
# its focal length through the take in its node's extras.
scene = bpy.context.scene
now = scene.frame_current
for o in scene.objects:
    data = o.data if o.type == "CAMERA" else None
    action = data and data.animation_data and data.animation_data.action
    if action:
        first, last = (int(f) for f in action.frame_range)
        frames = range(first, last + 1)
        lens = []
        for f in frames:
            scene.frame_set(f)
            lens.append(data.lens)
        scene.frame_set(now)
        o["lumit_lens"] = {
            "rest": data.lens,
            "times": [f / scene.render.fps for f in frames],
            "lens": lens,
        }

# Option names move between Blender versions, so only pass the ones this
# exporter has.
wanted = {
    "filepath": out,
    "export_format": "GLB",
    "export_apply": True,
    "export_animations": True,
    "export_cameras": True,
    "export_lights": True,
    "export_extras": True,
    "export_yup": True,
    "use_visible": True,
    "export_draco_mesh_compression_enable": False,
    "export_meshopt_compression_enable": False,
}
known = bpy.ops.export_scene.gltf.get_rna_type().properties.keys()
bpy.ops.export_scene.gltf(**{k: v for k, v in wanted.items() if k in known})
