use super::*;

/// A depth pack exactly as docs/impl/addons.md §4 writes it.
fn good() -> String {
    r#"{
      "format": 1,
      "id": "depth-anything-v2-small",
      "kind": "model",
      "name": "Depth Anything V2 Small",
      "version": "1.0",
      "summary": "A depth map from a single frame",
      "licence": "Apache-2.0",
      "licence_url": "https://example.invalid/licence",
      "size": 99060839,
      "requires": ["runtime"],
      "platforms": {
        "any": {
          "downloads": [
            { "url": "https://example.invalid/model.onnx",
              "sha256": "afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c",
              "size": 99060839,
              "unpack": "file",
              "dest": "model.onnx" }
          ]
        }
      },
      "model": {
        "task": "depth",
        "arch": "depth-anything",
        "file": "model.onnx",
        "input": "pixel_values",
        "output": "predicted_depth",
        "size": 518,
        "multiple": 14,
        "mean": [0.485, 0.456, 0.406],
        "std": [0.229, 0.224, 0.225],
        "output_kind": "inverse-relative"
      }
    }"#
    .to_string()
}

/// Swap one value in the written manifest for another, so each refusal
/// below differs from the good one in exactly one place.
fn with(from: &str, to: &str) -> String {
    let text = good();
    assert!(text.contains(from), "the fixture has no {from}");
    text.replace(from, to)
}

/// **A good manifest round trips.** Parsed, written back out and parsed
/// again gives the same thing, which is what lets an install copy the
/// catalogue's text into the folder unchanged.
#[test]
fn a_good_manifest_round_trips() {
    let parsed = parse(&good()).unwrap();
    assert_eq!(parsed.id, "depth-anything-v2-small");
    assert_eq!(parsed.kind, Kind::Model);
    assert_eq!(parsed.task(), Some(Task::Depth));
    assert_eq!(
        parsed.expected(),
        vec![("model.onnx".into(), Some(99_060_839))]
    );

    let written = serde_json::to_string(&parsed).unwrap();
    assert_eq!(parse(&written).unwrap(), parsed);
}

/// **Everything else §4 rules out is refused by name.** One case per
/// rule, each differing from the good manifest in one place.
#[test]
fn every_rule_in_the_format_refuses_by_name() {
    let cases = [
        (with(r#""kind": "model""#, r#""kind": "plugin""#), "kind"),
        (with(r#""task": "depth""#, r#""task": "painting""#), "task"),
        (
            with(
                r#""id": "depth-anything-v2-small""#,
                r#""id": "Depth Anything""#,
            ),
            "id",
        ),
        (
            with(r#""arch": "depth-anything""#, r#""arch": "midas""#),
            "arch",
        ),
        (
            with(r#""task": "depth""#, r#""task": "matte""#),
            "task against arch",
        ),
        (with(r#""any""#, r#""solaris-m68k""#), "platform key"),
        (
            with(
                r#""sha256": "afb6a5c28f3b6bf1618c6e43f02073ef9dfdc70e937502d51603e57b0a1df10c","#,
                "",
            ),
            "no digest",
        ),
        (
            with(r#""dest": "model.onnx""#, r#""dest": "../model.onnx""#),
            "dest outside the folder",
        ),
        (
            with(r#""unpack": "file""#, r#""unpack": "zip""#),
            "zip with no entries",
        ),
        (
            with(r#""unpack": "file""#, r#""unpack": "tar.gz""#),
            "tar.gz with no entries",
        ),
        (
            with(r#""kind": "model""#, r#""kind": "runtime""#),
            "runtime with a model block",
        ),
    ];
    for (text, what) in cases {
        let refusal = parse(&text).unwrap_err();
        assert!(
            matches!(refusal, MlError::Invalid(_)),
            "{what} was not refused: {refusal:?}"
        );
    }
}

/// The model blocks the catalogue publishes, copied from
/// `lumit-addons/addons/<id>/addon.json`, with the task each one claims.
///
/// The catalogue is a second repository and nothing in either tree reads
/// the other's files, so this is the only place a block written one way
/// and a struct written another are ever put together.
const PUBLISHED: [(&str, Task); 6] = [
    (
        r#""task": "synthesis", "arch": "rife", "file": "rife.onnx",
           "img0": "img0", "img1": "img1", "timestep": "timestep",
           "output": "output", "multiple": 32"#,
        Task::Synthesis,
    ),
    (
        r#""task": "depth", "arch": "depth-anything", "file": "model.onnx",
           "input": "pixel_values", "output": "predicted_depth",
           "size": 518, "multiple": 14,
           "mean": [0.485, 0.456, 0.406], "std": [0.229, 0.224, 0.225],
           "output_kind": "inverse-relative""#,
        Task::Depth,
    ),
    (
        r#""task": "matte", "arch": "rvm", "file": "rvm.onnx",
           "src": "src",
           "state_in": ["r1i", "r2i", "r3i", "r4i"],
           "state_out": ["r1o", "r2o", "r3o", "r4o"],
           "downsample": "downsample_ratio",
           "foreground": "fgr", "matte": "pha""#,
        Task::Matte,
    ),
    (
        r#""task": "matte", "arch": "birefnet", "file": "model.onnx",
           "input": "input_image", "output": "output_image", "size": 1024,
           "mean": [0.485, 0.456, 0.406], "std": [0.229, 0.224, 0.225]"#,
        Task::Matte,
    ),
    (
        r#""task": "segmentation", "arch": "sam2",
           "encoder": "encoder.onnx", "decoder": "decoder.onnx",
           "image": "image", "embed": "image_embed",
           "high_res": ["high_res_feats_0", "high_res_feats_1"],
           "point_coords": "point_coords", "point_labels": "point_labels",
           "mask_input": "mask_input", "has_mask_input": "has_mask_input",
           "masks": "masks", "iou": "iou_predictions", "size": 1024,
           "mean": [0.485, 0.456, 0.406], "std": [0.229, 0.224, 0.225]"#,
        Task::Segmentation,
    ),
    (
        r#""task": "speech", "arch": "whisper",
           "encoder": "encoder_model_fp16.onnx",
           "decoder": "decoder_model_merged_fp16.onnx",
           "tokenizer": "tokenizer.json", "generation": "generation_config.json",
           "config": "config.json", "translates": false,
           "vad": "silero_vad.onnx",
           "voices": "voices.onnx""#,
        Task::Speech,
    ),
];

/// **Every model block the catalogue publishes parses.** A key named one
/// way on one side and another on the other is how two of the five packs
/// once downloaded in full and then refused to install, and nothing in
/// either repository would have caught it.
#[test]
fn every_model_block_the_catalogue_publishes_parses() {
    for (model, task) in PUBLISHED {
        let text = format!(
            r#"{{"format":1,"id":"pack","kind":"model","name":"A pack",
                "version":"1","licence":"MIT","platforms":{{"any":{{"downloads":[
                  {{"url":"https://example.invalid/f",
                    "sha256":"0000000000000000000000000000000000000000000000000000000000000003",
                    "size":1,"unpack":"file","dest":"model.onnx"}}]}}}},
                "model":{{{model}}}}}"#
        );
        match parse(&text) {
            Ok(parsed) => assert_eq!(parsed.task(), Some(task), "{model}"),
            Err(why) => {
                panic!("the catalogue publishes a block this build refuses: {why}\n{model}")
            }
        }
    }
}
