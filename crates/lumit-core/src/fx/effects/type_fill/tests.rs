use super::{Piece, TypeFill};
use crate::fx::params::Params;
use crate::fx::registry::EffectMetadata;

/// Type fill: rows split the height, columns are as wide as
/// their words, a shape in the middle of a row leaves each column only
/// the parts outside it (inset by Word gap), a part narrower than Min
/// width stays empty, Invert region swaps inside and out, and the orders
/// move or shuffle the words. Guards words drawn over the shape, or
/// boxes that do not fill the room.
#[test]
fn words_fill_the_room_the_region_leaves() {
    let mut t = TypeFill::read(Params::EMPTY);
    t.rows = 2;
    t.row_gap = 10.0;
    t.word_gap = 10.0;
    t.min_width = 20.0;
    assert_eq!(t.word_list(), vec!["Flood", "the", "Grid"]);
    let bands = t.row_bands(110.0, 1.0);
    assert_eq!(bands, vec![[0.0, 50.0], [60.0, 110.0]]);

    // A row 400 px wide with the region everywhere but 150..250.
    let samples: Vec<f32> = (0..400)
        .map(|x| if (150..250).contains(&x) { 0.0 } else { 1.0 })
        .collect();
    let free = t.free_spans(&samples);
    assert_eq!(free, vec![[0.0, 150.0], [250.0, 400.0]]);
    // Words 2, 1 and 1 wide: columns 0..190, 200..295, 305..400.
    let natural = [2.0, 1.0, 1.0];
    let pieces = t.pieces(
        &bands[..1],
        std::slice::from_ref(&free),
        &natural,
        400.0,
        1.0,
    );
    assert_eq!(
        pieces,
        vec![
            Piece {
                word: 0,
                rect: [0.0, 0.0, 145.0, 50.0],
                clip: [0.0, 145.0]
            },
            Piece {
                word: 1,
                rect: [255.0, 0.0, 295.0, 50.0],
                clip: [255.0, 295.0]
            },
            Piece {
                word: 2,
                rect: [305.0, 0.0, 400.0, 50.0],
                clip: [305.0, 400.0]
            },
        ]
    );
    // A free stretch of 15 px is too narrow for a word.
    let narrow = vec![vec![[0.0, 15.0]]];
    assert!(t
        .pieces(&bands[..1], &narrow, &natural, 400.0, 1.0)
        .is_empty());

    // A shape at 230..250 cuts "the" (200..295) in two: one "the" across
    // the whole column, its left slice on one side, its right on the
    // other, never a second "the".
    let cut: Vec<f32> = (0..400)
        .map(|x| if (230..250).contains(&x) { 0.0 } else { 1.0 })
        .collect();
    let pieces = t.pieces(&bands[..1], &[t.free_spans(&cut)], &natural, 400.0, 1.0);
    assert_eq!(
        pieces[1..3],
        [
            Piece {
                word: 1,
                rect: [200.0, 0.0, 295.0, 50.0],
                clip: [200.0, 225.0]
            },
            Piece {
                word: 1,
                rect: [200.0, 0.0, 295.0, 50.0],
                clip: [255.0, 295.0]
            },
        ]
    );

    let mut inverted = t;
    inverted.invert = true;
    assert_eq!(inverted.free_spans(&samples), vec![[150.0, 250.0]]);

    // Cut words: every word whole at its column's width, wherever the
    // region is; the region then cuts it pixel by pixel, and a Type fill
    // and its inverted twin share each edge pixel exactly.
    let mut cut_out = t;
    cut_out.cut_words = true;
    let mut cut_in = inverted;
    cut_in.cut_words = true;
    let whole = cut_out.pieces(&bands[..1], &[vec![]], &natural, 400.0, 1.0);
    assert_eq!(
        whole.iter().map(|p| (p.word, p.clip)).collect::<Vec<_>>(),
        vec![(0, [0.0, 190.0]), (1, [200.0, 295.0]), (2, [305.0, 400.0])]
    );
    for v in [0.0, 0.3, 0.5, 0.7, 1.0] {
        let sum = cut_out.coverage(v) + cut_in.coverage(v);
        assert!((sum - 1.0).abs() < 1e-6, "edge pixel {v} shared");
    }
    assert!(cut_out.coverage(1.0) == 1.0 && cut_out.coverage(0.0) == 0.0);

    let mut shifted = t;
    shifted.order = 1;
    assert_eq!(shifted.row_order(1, 3), vec![1, 2, 0]);
    let mut random = t;
    random.order = 2;
    let a = random.row_order(4, 3);
    let mut sorted = a.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![0, 1, 2], "a shuffle of the words");
    assert_eq!(a, random.row_order(4, 3), "the same every frame");
}

/// Grid height stands the rows together in a band of that height in the
/// middle of the frame, so one row keyed into forty grows into a grid.
#[test]
fn grid_height_centres_the_rows() {
    let mut t = TypeFill::read(Params::EMPTY);
    t.rows = 2;
    t.row_gap = 0.0;
    t.grid_height = 40.0;
    assert_eq!(t.row_bands(100.0, 1.0), vec![[30.0, 50.0], [50.0, 70.0]]);
    t.grid_height = 0.0;
    assert_eq!(t.row_bands(100.0, 1.0), vec![[0.0, 50.0], [50.0, 100.0]]);
}

/// Fit piece: a shape that cuts a column in two leaves two pieces, each
/// with the whole word, fitted to its own box across and down at Piece
/// scale; a piece the region reaches only part of the way down gets a
/// short word. Guards a word cut by the shape, or stretched over it.
#[test]
fn fit_piece_gives_each_piece_its_own_whole_word() {
    let mut t = TypeFill::read(Params::EMPTY);
    t.words = crate::fx::params::ShortText::new("a");
    t.word_gap = 0.0;
    t.min_width = 1.0;
    t.piece_scale = 50.0;
    // One row 0..20 across 10 px: free at 0..4 all the way down, held
    // at 4..6, free at 6..10 only from 10 to 20.
    let mut row = vec![Some([0.0, 20.0]); 4];
    row.extend([None, None]);
    row.extend(vec![Some([10.0, 20.0]); 4]);
    let pieces = t.fit_pieces(&[row], &[1.0], 10.0, 1.0);
    assert_eq!(
        pieces,
        vec![
            Piece {
                word: 0,
                rect: [1.0, 5.0, 3.0, 15.0],
                clip: [1.0, 3.0]
            },
            Piece {
                word: 0,
                rect: [7.0, 12.5, 9.0, 17.5],
                clip: [7.0, 9.0]
            },
        ]
    );
}

/// Column widths sets each column's share of the width; a list that is
/// not one positive number for each word is left out.
#[test]
fn column_widths_set_the_columns() {
    let mut t = TypeFill::read(Params::EMPTY);
    assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![2.0, 1.0, 2.0]);
    t.column_widths = crate::fx::params::ShortText::new("46|8|40");
    assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![46.0, 8.0, 40.0]);
    t.column_widths = crate::fx::params::ShortText::new("46-8 40");
    assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![46.0, 8.0, 40.0]);
    // A shorter list goes round again.
    t.column_widths = crate::fx::params::ShortText::new("46|8");
    assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![46.0, 8.0, 46.0]);
    t.column_widths = crate::fx::params::ShortText::new("1");
    assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![1.0, 1.0, 1.0]);
    // A number that is not above 0 leaves the list out.
    t.column_widths = crate::fx::params::ShortText::new("46|x");
    assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![2.0, 1.0, 2.0]);
    t.column_widths = crate::fx::params::ShortText::new("46|0");
    assert_eq!(t.shares(&[2.0, 1.0, 2.0]), vec![2.0, 1.0, 2.0]);
}

/// Direction: Columns renames the rows that then work down the columns,
/// and Rows keeps their own names.
#[test]
fn columns_rename_the_rows() {
    use crate::fx::registry::EffectDef;
    let mut inst = crate::fx::builtins::instantiate("type_fill").unwrap();
    assert!(super::TypeFillDef.row_state(&inst).labels.is_empty());
    inst.params.retain(|p| p.id != "direction");
    inst.params.push(crate::model::EffectParam {
        id: "direction".to_owned(),
        value: crate::model::EffectValue::Choice(1),
        extra: serde_json::Map::new(),
    });
    let labels = super::TypeFillDef.row_state(&inst).labels;
    assert!(labels.contains(&("rows", "Columns")));
    assert!(labels.contains(&("row_shift", "Column shift")));
    assert!(labels.contains(&("grid_height", "Grid width")));
}

/// Repeat words sets the list one after another that many times.
#[test]
fn repeat_words_sets_the_list_again() {
    let mut t = TypeFill::read(Params::EMPTY);
    t.words = crate::fx::params::ShortText::new("a|b");
    assert_eq!(t.word_list(), vec!["a", "b"]);
    t.repeat_words = 3;
    assert_eq!(t.word_list(), vec!["a", "b", "a", "b", "a", "b"]);
}

/// Column mix goes from the words' own widths (0 %) to Column widths
/// (100 %) through shares of one. Guards a jump when it is keyed.
#[test]
fn column_mix_slides_between_the_widths() {
    let mut t = TypeFill::read(Params::EMPTY);
    t.column_widths = crate::fx::params::ShortText::new("1|1");
    let natural = [3.0, 1.0];
    t.column_mix = 0.0;
    assert_eq!(t.shares(&natural), vec![0.75, 0.25]);
    t.column_mix = 50.0;
    assert_eq!(t.shares(&natural), vec![0.625, 0.375]);
    t.column_mix = 100.0;
    assert_eq!(t.shares(&natural), vec![1.0, 1.0]);
}

/// Row open grows every row out of a flat line through its middle; with
/// Row stagger the middle row opens first and the outer rows last.
#[test]
fn row_open_and_stagger_open_the_rows_from_the_middle() {
    let mut t = TypeFill::read(Params::EMPTY);
    t.rows = 3;
    t.row_gap = 0.0;
    t.row_open = 50.0;
    assert_eq!(
        t.row_bands(90.0, 1.0),
        vec![[7.5, 22.5], [37.5, 52.5], [67.5, 82.5]]
    );
    t.row_stagger = 50.0;
    // The middle row is open, the outer ones still flat.
    assert_eq!(
        t.row_bands(90.0, 1.0),
        vec![[15.0, 15.0], [30.0, 60.0], [75.0, 75.0]]
    );
    t.row_open = 75.0;
    assert!((t.row_openness(0, 3) - 0.5).abs() < 1e-6);
    assert!((t.row_openness(2, 3) - 0.5).abs() < 1e-6);
    t.row_open = 100.0;
    assert_eq!(t.row_bands(90.0, 1.0)[0], [0.0, 30.0], "all open at 100 %");
}

/// Row shift slides a row's columns and wraps them round every width
/// plus one Word gap; Alternate rows slides the odd rows the other way.
/// Guards a gap opening at the edge as the row scrolls.
#[test]
fn row_shift_scrolls_the_rows_round() {
    let mut t = TypeFill::read(Params::EMPTY);
    t.words = crate::fx::params::ShortText::new("a|b");
    t.word_gap = 0.0;
    t.min_width = 1.0;
    let natural = [1.0, 1.0];
    let full = |t: &TypeFill, r: usize| {
        let reach = vec![vec![Some([0.0, 10.0]); 100]; r + 1];
        t.fit_pieces(&reach, &natural, 100.0, 1.0)
            .into_iter()
            .filter(|p| p.rect[1] >= 0.0)
            .map(|p| (p.word, p.clip))
            .collect::<Vec<_>>()
    };
    t.piece_scale = 100.0;
    t.row_shift = 20.0;
    // "b" wraps in from the left: 0..20, then "a" 20..70, "b" 70..100.
    let first: Vec<_> = full(&t, 0);
    assert_eq!(
        first,
        vec![(1, [0.0, 20.0]), (0, [20.0, 70.0]), (1, [70.0, 100.0])]
    );
    t.row_shift = 120.0;
    assert_eq!(full(&t, 0), first, "a whole width round is the same row");
    t.row_shift = 20.0;
    t.alternate_rows = true;
    assert_eq!(t.row_offset(0, 1.0), 20.0);
    assert_eq!(t.row_offset(1, 1.0), -20.0);
    assert_eq!(t.row_offset(1, 2.0), -40.0, "px@comp");
}
